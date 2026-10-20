//! The act DRLG: the `Drlg` that owns one act's levels and rooms, the level list and
//! level seeds, vis/warp records, coordinate lookups, the level lifecycle (activity
//! counts, freeing idle levels) and the spawn-room choice.
//!
//! The per-type generators (maze, preset, outdoor) live behind the `LevelTypes` trait;
//! this file drives them. Rooms, tiles and active rooms are in the sibling modules.

use std::collections::BTreeMap;

use crate::rng::Seed;
use crate::units::RoomId;

use super::data::DrlgData;
use super::room::DrlgRoom;
use super::seams::{LevelTypes, Services};
use super::tiles::Dt1File;
use super::{is_town, room_flags, DrlgError, DrlgRoomId, LevelIdx, TileRect, SUBTILES};

/// Level flag 0x10: automap reveal, set on every level of a client copy.
pub const LEVEL_FLAG_CLIENT: u32 = 0x10;

/// Inactive-frame reset of a level: it must see 10 passes of tick step 10 with no
/// activity before its rooms are freed.
pub const LEVEL_INACTIVITY_RESET: i32 = 10;

/// Tomb base level of Act II: the tomb levels are 66..72.
pub const TOMB_BASE: u32 = 66;

/// Spawn-tile class table: (a, b) for spawn-tile index t = 0..13. Indices with `a` ≠ 0
/// also match records whose class `b` is the same.
pub const SPAWN_TILE_CLASSES: [(u32, u32); 14] = [
    (1, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (1, 1),
    (0, 1),
    (0, 1),
    (0, 1),
    (0, 1),
    (0, 2),
    (0, 3),
    (0, 4),
    (0, 5),
];

/// Spawn-tile index of the location monster population measures warp distance from.
pub const KIND11_TILE: u32 = 11;

/// Spawn-tile index that means "the waypoint room".
pub const SPAWN_WAYPOINT: u32 = 13;

/// One level of an act: identity, rect, seed, activity and its spawn and warp data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    /// Level id.
    pub id: u32,
    /// DRLG type: 1 maze, 2 preset, 3 outdoor.
    pub drlg_type: u32,
    /// Level type: `lvltypes` row.
    pub level_type: u32,
    /// Flags (0x10 automap reveal).
    pub flags: u32,
    /// Position and size in tiles.
    pub rect: TileRect,
    /// Level seed.
    pub seed: Seed,
    /// Activity count: how many clients are in or next to the level.
    pub activity: i32,
    /// Inactive frames left before an idle level may be freed.
    pub inactive_frames: i32,
    /// Spawn-tile records, set by the level-type generators.
    pub spawn_tiles: Vec<SpawnTile>,
    /// Warp-room centres in sub-tiles.
    pub warp_centres: Vec<(i32, i32)>,
    /// Populated-room memory, one entry per room in list order; remembers which rooms were
    /// populated when the level's rooms were freed.
    pub populated_memory: Option<Vec<bool>>,
    /// Coordinate-list counter: zero at allocation, not reset when the level's rooms are
    /// freed.
    pub coord_counter: u32,
    pub(super) first_room: Option<DrlgRoomId>,
    pub(super) room_count: u32,
    pub(super) next: Option<LevelIdx>,
}

/// A spawn-tile record (stride 12: x, y, tile index), tile coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpawnTile {
    /// Tile x.
    pub x: i32,
    pub y: i32,
    /// Spawn-tile index.
    pub index: u32,
}

/// A DRLG's editable vis/warp copy of one level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarpRecord {
    /// The level this record belongs to.
    pub level_id: u32,
    /// Neighbouring level ids, 0 = none.
    pub vis: [u32; 8],
    /// Warp ids, −1 = none.
    pub warp: [i32; 8],
}

/// The waypoint room and the position of its waypoint object, if any.
type WaypointRoom = (DrlgRoomId, Option<(i32, i32)>);

/// Where a unit spawns in a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnPoint {
    /// The DRLG room.
    pub room: DrlgRoomId,
    /// Tile x position.
    pub x: i32,
    pub y: i32,
    /// The room's active room after streaming, if it has one.
    pub active: Option<RoomId>,
}

/// The client build cursor: where the incremental room build of a client copy resumes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuildCursor {
    /// Zero at allocation: the first build step moves it to the first status-2 room.
    #[default]
    None,
    /// The status-2 list's head node, which reads as status 2 and so is kept by the first
    /// build step.
    Head,
    /// A DRLG room.
    Room(DrlgRoomId),
}

/// One act's DRLG and everything it owns: levels, rooms, vis/warp records and tile caches.
#[derive(Clone, Debug)]
pub struct Drlg {
    /// Act number, 0-based.
    pub act: u8,
    /// DRLG seed.
    pub seed: Seed,
    /// `dwStartSeed`: the first draw of the DRLG seed; each level seed derives from it.
    pub start_seed: u32,
    /// Copy of the init seed.
    pub init_seed: u32,
    /// Difficulty.
    pub difficulty: u8,
    /// Whether this is a client copy.
    pub on_client: bool,
    /// Act II staff tomb level, 0 elsewhere.
    pub staff_tomb: u32,
    /// Act II boss tomb level, 0 elsewhere.
    pub boss_tomb: u32,
    /// Act III jungle-link bit.
    pub jungle_link: bool,
    /// Rooms built (statistic).
    pub rooms_built: u32,
    /// Builds since the last client update.
    pub builds_since_update: u8,
    /// Client build timer.
    pub build_timer: u8,
    /// Client build cursor: none at allocation, a DRLG room, or the status-2 list's head
    /// node.
    pub build_cursor: BuildCursor,
    /// Freed-room counter.
    pub freed_rooms: u32,
    /// Automap callback calls of a client copy, in call order: each streamed room of a
    /// whole preset level. The callback is presentation: the client reads the list and adds
    /// the rooms' cells.
    pub automap_rooms: Vec<DrlgRoomId>,
    /// Town-automap callback calls of a client copy, in call order: (level id, picked file,
    /// centre tile). Presentation, read by the client.
    pub automap_towns: Vec<(u32, u32, (i32, i32))>,
    pub(super) levels: Vec<Level>,
    pub(super) level_head: Option<LevelIdx>,
    /// Vis/warp records, head first.
    pub(super) warp_records: Vec<WarpRecord>,
    pub(super) rooms: Vec<Option<DrlgRoom>>,
    /// Room status lists 0..3, head first.
    pub(super) status_lists: [Vec<DrlgRoomId>; 4],
    /// Active room → DRLG room (lookup only).
    pub(super) active_index: BTreeMap<RoomId, DrlgRoomId>,
    /// DT1 files loaded by any room, by path (process-wide cache).
    pub(super) dt1_files: Vec<Dt1File>,
    pub(super) dt1_by_path: BTreeMap<Vec<u8>, u32>,
    /// The game's view gap ([`super::room::view_gap_for_width`]): the near
    /// gap in a 4:3 game; wider views also load the rooms' view extras
    /// ([`Drlg::view_extras`]).
    pub view_gap: i32,
    /// The view extras held at status 1 for each room a client entered,
    /// one list per hold (a server DRLG of a wider view only).
    pub(super) view_holds: BTreeMap<DrlgRoomId, Vec<Vec<DrlgRoomId>>>,
}

impl Drlg {
    /// An empty DRLG with its seeds set; no draw.
    fn alloc(act: u8, init_seed: u32, difficulty: u8, on_client: bool) -> Self {
        Self {
            act,
            seed: Seed::init_low(init_seed),
            start_seed: 0,
            init_seed,
            difficulty,
            on_client,
            staff_tomb: 0,
            boss_tomb: 0,
            jungle_link: false,
            rooms_built: 0,
            builds_since_update: 0,
            build_timer: 0,
            build_cursor: BuildCursor::None,
            freed_rooms: 0,
            automap_rooms: Vec::new(),
            automap_towns: Vec::new(),
            levels: Vec::new(),
            level_head: None,
            warp_records: Vec::new(),
            rooms: Vec::new(),
            status_lists: Default::default(),
            active_index: BTreeMap::new(),
            dt1_files: Vec::new(),
            dt1_by_path: BTreeMap::new(),
            view_gap: super::room::NEAR_GAP,
            view_holds: BTreeMap::new(),
        }
    }

    /// Creates an act's DRLG. `town_level` is the server's town level id (0 for a client
    /// copy); `on_client` the client flag. The tile library cache draws nothing and is the
    /// tile source's job.
    pub fn create(
        act: u8,
        init_seed: u32,
        difficulty: u8,
        town_level: u32,
        on_client: bool,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
    ) -> Result<Self, DrlgError> {
        let mut d = Self::alloc(act, init_seed, difficulty, on_client);
        // First draw: dwStartSeed.
        d.start_seed = d.seed.step();
        // Act-specific draws.
        match act {
            1 => loop {
                let a = d.seed.step() % 7;
                let b = d.seed.step() % 7;
                if a != b {
                    d.staff_tomb = TOMB_BASE + a;
                    d.boss_tomb = TOMB_BASE + b;
                    break;
                }
            },
            2 => d.jungle_link = d.seed.step() & 1 != 0,
            _ => {}
        }
        // The status lists start as empty Vecs. The act placer lays out the act's levels.
        types.create_act_levels(&mut d, data)?;
        // The server's town is generated up front.
        if town_level != 0 {
            let l = d.get_or_alloc_level(data, types, town_level)?;
            d.generate_level(data, types, l)?;
        }
        Ok(d)
    }

    // ---- levels -----------------------------------------------------------

    /// The level at a list index.
    pub fn level(&self, idx: LevelIdx) -> &Level {
        &self.levels[idx.0 as usize]
    }

    /// The level at a list index, mutable.
    pub fn level_mut(&mut self, idx: LevelIdx) -> &mut Level {
        &mut self.levels[idx.0 as usize]
    }

    /// The level with id `id` if allocated (list walk, no allocation).
    pub fn find_level(&self, id: u32) -> Option<LevelIdx> {
        let mut cur = self.level_head;
        while let Some(l) = cur {
            if self.level(l).id == id {
                return Some(l);
            }
            cur = self.level(l).next;
        }
        None
    }

    /// The level list, head first (newest allocation first).
    pub fn level_list(&self) -> Vec<LevelIdx> {
        let mut out = Vec::new();
        let mut cur = self.level_head;
        while let Some(l) = cur {
            out.push(l);
            cur = self.level(l).next;
        }
        out
    }

    /// Returns the level with id `id`, allocating it if absent. Never generates rooms.
    pub fn get_or_alloc_level(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        id: u32,
    ) -> Result<LevelIdx, DrlgError> {
        match self.find_level(id) {
            Some(l) => Ok(l),
            None => self.alloc_level(data, types, id),
        }
    }

    /// Allocates a level: seed `init_low(dwStartSeed + id)`, type init, then prepend to
    /// the level list.
    fn alloc_level(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        id: u32,
    ) -> Result<LevelIdx, DrlgError> {
        let def = data.level(id)?;
        let idx = LevelIdx(self.levels.len() as u32);
        self.levels.push(Level {
            id,
            drlg_type: def.drlg_type,
            level_type: def.level_type,
            flags: if self.on_client { LEVEL_FLAG_CLIENT } else { 0 },
            rect: TileRect::default(),
            seed: Seed::init_low(self.start_seed.wrapping_add(id)),
            activity: 0,
            inactive_frames: 0,
            spawn_tiles: Vec::new(),
            warp_centres: Vec::new(),
            populated_memory: None,
            coord_counter: 0,
            first_room: None,
            room_count: 0,
            next: None,
        });
        // Unknown DRLG types get no init, silently.
        if matches!(def.drlg_type, 1..=3) {
            types.init_level(self, data, idx)?;
            if matches!(def.drlg_type, 1 | 2) {
                self.set_level_position_and_size(data, types, idx)?;
            }
        }
        let head = self.level_head.replace(idx);
        self.level_mut(idx).next = head;
        Ok(idx)
    }

    /// Sets a level's rect: size from `SizeX/Y[difficulty]`, position = the `Depend`
    /// level's position (allocated if needed) + offset.
    pub fn set_level_position_and_size(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        idx: LevelIdx,
    ) -> Result<(), DrlgError> {
        let def = data.level(self.level(idx).id)?.clone();
        let (w, h) = def.size[(self.difficulty as usize).min(2)];
        let (mut x, mut y) = (0, 0);
        if def.depend != 0 {
            let d = self.get_or_alloc_level(data, types, def.depend)?;
            x = self.level(d).rect.x;
            y = self.level(d).rect.y;
        }
        self.level_mut(idx).rect = TileRect::new(
            x.wrapping_add(def.offset.0),
            y.wrapping_add(def.offset.1),
            w,
            h,
        );
        Ok(())
    }

    /// Generates a level's rooms, without the client automap streaming (that needs the
    /// room services: [`Drlg::generate_level_svc`]).
    pub fn generate_level(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        idx: LevelIdx,
    ) -> Result<(), DrlgError> {
        self.generate_rooms(data, types, idx)?;
        self.generate_finish(data, idx)
    }

    /// Generates a level's rooms with the client automap streaming. On a client copy
    /// (`on_client`) and a level whose type asks for it ([`LevelTypes::automap_streams`]),
    /// the type generator then generates the level's Vis levels that have no rooms and
    /// streams every room of the level in list order (their seed resets and tile draws),
    /// before the populated-room memory and warp-room centres are set.
    pub fn generate_level_svc(
        &mut self,
        svc: &mut Services<'_>,
        idx: LevelIdx,
    ) -> Result<(), DrlgError> {
        self.generate_rooms(svc.data, svc.types, idx)?;
        if self.on_client && svc.types.automap_streams(self, idx) {
            let id = self.level(idx).id;
            for vis in self.vis_array(svc.data, id)? {
                if vis == 0 {
                    continue;
                }
                let v = self.get_or_alloc_level(svc.data, svc.types, vis)?;
                if self.level(v).first_room.is_none() {
                    self.generate_level_svc(svc, v)?;
                }
            }
            for r in self.level_rooms(idx) {
                if self.stream_room(svc, r)?.is_some() {
                    self.automap_rooms.push(r);
                }
            }
        }
        self.generate_finish(svc.data, idx)
    }

    /// Resets the level seed and runs the type generator.
    fn generate_rooms(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        idx: LevelIdx,
    ) -> Result<(), DrlgError> {
        let id = self.level(idx).id;
        self.level_mut(idx).seed = Seed::init_low(self.start_seed.wrapping_add(id));
        if matches!(self.level(idx).drlg_type, 1..=3) {
            types.generate(self, data, idx)?;
        }
        Ok(())
    }

    /// Restores populated-room memory and records warp-room centres.
    fn generate_finish(&mut self, data: &DrlgData, idx: LevelIdx) -> Result<(), DrlgError> {
        let id = self.level(idx).id;
        // Populated-room memory.
        if let Some(mem) = self.level(idx).populated_memory.clone() {
            for (i, r) in self.level_rooms(idx).into_iter().enumerate() {
                // More rooms than memory entries read as not populated; this cannot happen
                // while a regenerated level repeats its room count.
                if mem.get(i).copied().unwrap_or(false) {
                    self.room_mut(r).other_flags |= 1;
                }
            }
        }
        // Warp-room centres.
        let warp = self.warp_array(data, id)?;
        for r in self.level_rooms(idx) {
            let room = self.room(r);
            let qualifies = room.flags & room_flags::ANY_WAYPOINT != 0
                || (0..8).any(|i| room.flags & (room_flags::WARP_0 << i) != 0 && warp[i] != -1);
            if qualifies {
                let c = (
                    (room.rect.x + room.rect.w / 2) * SUBTILES,
                    (room.rect.y + room.rect.h / 2) * SUBTILES,
                );
                // The list is not bounded here.
                self.level_mut(idx).warp_centres.push(c);
            }
        }
        Ok(())
    }

    /// Rooms of a level in list order.
    pub fn level_rooms(&self, idx: LevelIdx) -> Vec<DrlgRoomId> {
        let mut out = Vec::new();
        let mut cur = self.level(idx).first_room;
        while let Some(r) = cur {
            out.push(r);
            cur = self.room(r).next;
        }
        out
    }

    /// Number of rooms in a level.
    pub fn room_count(&self, idx: LevelIdx) -> u32 {
        self.level(idx).room_count
    }

    // ---- vis and warp records ---------------------------------------------

    fn record_pos(&self, level_id: u32) -> Option<usize> {
        self.warp_records
            .iter()
            .position(|r| r.level_id == level_id)
    }

    /// The vis/warp record of a level, created from leveldefs (prepended) if absent.
    pub fn warp_record_mut(
        &mut self,
        data: &DrlgData,
        level_id: u32,
    ) -> Result<&mut WarpRecord, DrlgError> {
        let pos = match self.record_pos(level_id) {
            Some(p) => p,
            None => {
                let def = data.level(level_id)?;
                self.warp_records.insert(
                    0,
                    WarpRecord {
                        level_id,
                        vis: def.vis,
                        warp: def.warp,
                    },
                );
                0
            }
        };
        Ok(&mut self.warp_records[pos])
    }

    /// The records, head first.
    pub fn warp_records(&self) -> &[WarpRecord] {
        &self.warp_records
    }

    /// The vis array of a level: its record if one exists, else the leveldefs row. A
    /// record for level 0 is an error.
    pub fn vis_array(&self, data: &DrlgData, level_id: u32) -> Result<[u32; 8], DrlgError> {
        match self.record_pos(level_id) {
            Some(_) if level_id == 0 => Err(DrlgError::WarpRecordLevelZero),
            Some(p) => Ok(self.warp_records[p].vis),
            None => Ok(data.level(level_id)?.vis),
        }
    }

    /// The warp array of a level: its record if one exists, else the leveldefs row. A
    /// record for level 0 is an error.
    pub fn warp_array(&self, data: &DrlgData, level_id: u32) -> Result<[i32; 8], DrlgError> {
        match self.record_pos(level_id) {
            Some(_) if level_id == 0 => Err(DrlgError::WarpRecordLevelZero),
            Some(p) => Ok(self.warp_records[p].warp),
            None => Ok(data.level(level_id)?.warp),
        }
    }

    /// The warp id in slot `slot` of a level.
    pub fn warp_id(&self, data: &DrlgData, level_id: u32, slot: usize) -> Result<i32, DrlgError> {
        Ok(self.warp_array(data, level_id)?[slot])
    }

    /// Sets a level's warp to the neighbour `vis` (D2MOO `DRLG_SetWarpId`). `slot` −1 =
    /// any free slot. With no matching vis and no free slot, vis goes to the record's
    /// level-id field and warp to vis[7].
    pub fn set_warp(
        &mut self,
        data: &DrlgData,
        level_id: u32,
        vis: u32,
        warp: i32,
        slot: i32,
    ) -> Result<(), DrlgError> {
        let rec = self.warp_record_mut(data, level_id)?;
        if let Some(i) = rec.vis.iter().position(|&v| v == vis) {
            rec.warp[i] = warp;
            return Ok(());
        }
        let mut slot = slot;
        if slot == -1 {
            slot = (0..8)
                .find(|&i| rec.vis[i] == 0 && rec.warp[i] == -1)
                .map_or(-1, |i| i as i32);
        }
        if slot == -1 {
            // Index −1 addresses the field before vis[0] (the level id) and
            // warp[−1] = vis[7].
            rec.level_id = vis;
            rec.vis[7] = warp as u32;
        } else {
            rec.vis[slot as usize] = vis;
            rec.warp[slot as usize] = warp;
        }
        Ok(())
    }

    /// The lvlwarp row of slot `slot` of a level.
    pub fn lvlwarp_row(
        &self,
        data: &DrlgData,
        level_id: u32,
        slot: usize,
        dir: u8,
    ) -> Result<usize, DrlgError> {
        data.lvlwarp_row(self.warp_id(data, level_id, slot)?, dir)
    }

    // ---- coordinates -------------------------------------------------------

    /// First level of the list whose rect contains the point.
    pub fn level_at(&self, x: i32, y: i32) -> Option<LevelIdx> {
        self.level_list()
            .into_iter()
            .find(|&l| self.level(l).rect.contains(x, y))
    }

    /// The room at tile (x, y), with an optional hint room and level. May allocate level
    /// 0 and generate a level.
    pub fn room_at(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        x: i32,
        y: i32,
        hint: Option<DrlgRoomId>,
        level: Option<LevelIdx>,
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        if let Some(h) = hint {
            if self.room(h).rect.contains(x, y) {
                return Ok(Some(h));
            }
            if let Some(near) = self.room(h).near.clone() {
                if let Some(&r) = near
                    .iter()
                    .find(|&&r| r != h && self.room(r).rect.contains(x, y))
                {
                    return Ok(Some(r));
                }
            }
        }
        let l = match level.or_else(|| self.level_at(x, y)) {
            Some(l) => l,
            None => self.get_or_alloc_level(data, types, 0)?,
        };
        if self.level(l).first_room.is_none() {
            self.generate_level(data, types, l)?;
        }
        Ok(self
            .level_rooms(l)
            .into_iter()
            .find(|&r| self.room(r).rect.contains(x, y)))
    }

    // ---- level lifecycle ----------------------------------------------------

    /// Updates activity counts for a client moving from `old` to `new` (either may be
    /// none): the old level and its Vis neighbours lose one, the new ones gain one.
    pub fn update_level_activity(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        old: Option<DrlgRoomId>,
        new: Option<DrlgRoomId>,
    ) -> Result<(), DrlgError> {
        let ol = old.map(|r| self.room(r).level);
        let nl = new.map(|r| self.room(r).level);
        if ol == nl {
            return Ok(());
        }
        for (lvl, delta) in [(ol, -1), (nl, 1)] {
            let Some(l) = lvl else { continue };
            self.touch_level(l, delta);
            let id = self.level(l).id;
            for v in self.vis_array(data, id)? {
                if v != 0 {
                    let vl = self.get_or_alloc_level(data, types, v)?;
                    self.touch_level(vl, delta);
                }
            }
        }
        Ok(())
    }

    /// Adds `delta` to the activity count (clamped to ≥ 0) and resets inactive frames to 10.
    fn touch_level(&mut self, l: LevelIdx, delta: i32) {
        let lv = self.level_mut(l);
        lv.activity = (lv.activity + delta).max(0);
        lv.inactive_frames = LEVEL_INACTIVITY_RESET;
    }

    /// Tick step 10: frees the rooms of inactive levels.
    pub fn free_inactive_levels(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
    ) -> Result<(), DrlgError> {
        let mut cur = self.level_head;
        while let Some(l) = cur {
            if self.level(l).activity == 0 && self.level(l).first_room.is_some() {
                if self.level(l).inactive_frames > 0 {
                    self.level_mut(l).inactive_frames -= 1;
                } else if self.level_free_test(data, types, l)? {
                    self.free_level_rooms(types, l);
                } else {
                    self.level_mut(l).inactive_frames = LEVEL_INACTIVITY_RESET;
                }
            }
            cur = self.level(l).next;
        }
        Ok(())
    }

    fn room_in_use(&self, r: DrlgRoomId) -> bool {
        let room = self.room(r);
        room.status < 4 || room.flags & room_flags::HAS_ROOM != 0
    }

    /// Whether a level's rooms may be freed: none in use, and no room of a Vis neighbour
    /// that links back to it in use. Clears the warp links and near arrays of those linked
    /// rooms on the way.
    pub fn level_free_test(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        l: LevelIdx,
    ) -> Result<bool, DrlgError> {
        if self.level_rooms(l).into_iter().any(|r| self.room_in_use(r)) {
            return Ok(false);
        }
        let id = self.level(l).id;
        for v in self.vis_array(data, id)? {
            if v == 0 {
                continue;
            }
            let vl = self.get_or_alloc_level(data, types, v)?;
            if self.level(vl).first_room.is_none() {
                continue;
            }
            let mut mask = 0;
            for (j, &x) in self.vis_array(data, v)?.iter().enumerate() {
                if x == id {
                    mask |= room_flags::WARP_0 << j;
                }
            }
            let linked: Vec<_> = self
                .level_rooms(vl)
                .into_iter()
                .filter(|&r| self.room(r).flags & mask != 0)
                .collect();
            if linked.iter().any(|&r| self.room_in_use(r)) {
                return Ok(false);
            }
            for r in linked {
                let room = self.room_mut(r);
                room.warp_links.clear();
                room.near = None;
            }
        }
        Ok(true)
    }

    /// Frees every room of a level: remembers populated bits, resets type data. The level
    /// stays in the list.
    pub fn free_level_rooms(&mut self, types: &mut dyn LevelTypes, l: LevelIdx) {
        let rooms = self.level_rooms(l);
        let bits: Vec<bool> = rooms
            .iter()
            .map(|&r| self.room(r).other_flags & 1 != 0)
            .collect();
        let mem = self
            .level_mut(l)
            .populated_memory
            .get_or_insert_with(|| vec![false; bits.len()]);
        // The memory is allocated once; a later free with more rooms than the first would
        // drop the extra bits. The room count repeats, so this does not occur.
        for (m, b) in mem.iter_mut().zip(bits) {
            *m = b;
        }
        for r in rooms {
            self.free_room(r);
        }
        types.reset_level(self, l);
        let lv = self.level_mut(l);
        lv.first_room = None;
        lv.room_count = 0;
        lv.spawn_tiles.clear();
        lv.warp_centres.clear();
    }

    // ---- population queries ------------------------------------------------

    /// The level id of a room, 0 for a room with flag 0x800000 (no population).
    pub fn populated_level(&self, id: DrlgRoomId) -> u32 {
        let r = self.room(id);
        if r.flags & room_flags::NO_POPULATION != 0 {
            return 0;
        }
        self.level(r.level).id
    }

    /// The number of rooms of `level_id` without flag 0x800000. The level is allocated if
    /// absent (allocation generates no rooms).
    pub fn populated_room_count(
        &mut self,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
        level_id: u32,
    ) -> Result<u32, DrlgError> {
        let l = self.get_or_alloc_level(data, types, level_id)?;
        Ok(self
            .level_rooms(l)
            .into_iter()
            .filter(|&r| self.room(r).flags & room_flags::NO_POPULATION == 0)
            .count() as u32)
    }

    /// [`Self::populated_room_count`] without allocating: an absent level has no rooms, so
    /// it counts 0.
    pub fn populated_room_count_if_present(&self, level_id: u32) -> u32 {
        self.find_level(level_id).map_or(0, |l| {
            self.level_rooms(l)
                .into_iter()
                .filter(|&r| self.room(r).flags & room_flags::NO_POPULATION == 0)
                .count() as u32
        })
    }

    /// The warp-room centres of the room's own level, in sub-tiles.
    pub fn warp_points(&self, id: DrlgRoomId) -> &[(i32, i32)] {
        &self.level(self.room(id).level).warp_centres
    }

    /// The spawn-room choice with tile index 11 and all its effects (generation, the
    /// level-seed draws, the streamed room). Returns the tile position, (−1, −1) when no
    /// room was found.
    pub fn kind11_location(
        &mut self,
        svc: &mut Services<'_>,
        level_id: u32,
    ) -> Result<(i32, i32), DrlgError> {
        match self.spawn_room(svc, level_id, KIND11_TILE) {
            Ok(p) => Ok((p.x, p.y)),
            Err(DrlgError::NoSpawnRoom) => Ok((-1, -1)),
            Err(e) => Err(e),
        }
    }

    // ---- spawn room ----------------------------------------------------------

    /// The spawn room and position of a level for spawn-tile index `tile`; the chosen room
    /// is streamed (made active).
    pub fn spawn_room(
        &mut self,
        svc: &mut Services<'_>,
        level_id: u32,
        tile: u32,
    ) -> Result<SpawnPoint, DrlgError> {
        let l = self.get_or_alloc_level(svc.data, svc.types, level_id)?;
        if self.level(l).first_room.is_none() {
            self.generate_level_svc(svc, l)?;
        }
        let mut pos: Option<(i32, i32)> = None;
        let room;
        // (x, y) default to (−1, −1); the room-centre default applies only on the
        // `Position` = 0 path.
        let mut default_centre = true;
        if svc.data.level(level_id)?.position != 0 {
            default_centre = false;
            if tile == SPAWN_WAYPOINT {
                let (r, p) = self
                    .waypoint_room(svc, l)?
                    .ok_or(DrlgError::NoWaypointRoom)?;
                room = r;
                pos = p;
            } else {
                let rec = self.pick_spawn_tile(l, tile);
                pos = Some((rec.x, rec.y));
                room = self
                    .room_at(svc.data, svc.types, rec.x, rec.y, None, None)?
                    .ok_or(DrlgError::NoSpawnRoom)?;
            }
        } else if let Some((r, p)) = self.waypoint_room(svc, l)? {
            room = r;
            pos = p;
        } else if let Some(r) = self.first_warp_room(svc.data, l)? {
            room = r;
        } else {
            let rect = self.level(l).rect;
            let (cx, cy) = (rect.x + rect.w / 2 - 2, rect.y + rect.h / 2 - 2);
            match self.room_at(svc.data, svc.types, cx, cy, None, Some(l))? {
                Some(r) => room = r,
                None => {
                    // Last resort: a random room, roll(room count) on the level seed.
                    let n = self.room_count(l) as i32;
                    let k = self.level_mut(l).seed.roll(n) as usize;
                    room = *self.level_rooms(l).get(k).ok_or(DrlgError::NoSpawnRoom)?;
                }
            }
        }
        let (x, y) = pos.unwrap_or_else(|| {
            let r = self.room(room).rect;
            if default_centre {
                (r.x + r.w / 2, r.y + r.h / 2)
            } else {
                (-1, -1)
            }
        });
        let active = self.stream_room(svc, room)?;
        Ok(SpawnPoint { room, x, y, active })
    }

    /// The record for spawn-tile index `t`. A record matches when its index equals `t`, or
    /// when `t`'s class pair has `a ≠ 0` and the record's `b` equals `t`'s. With `n` > 0
    /// matches, `roll(n)` on the level seed picks one; with none, record 0 (all zero if
    /// the level has no records).
    fn pick_spawn_tile(&mut self, l: LevelIdx, t: u32) -> SpawnTile {
        let class = |i: u32| SPAWN_TILE_CLASSES.get(i as usize).copied();
        let matches = |rec: &SpawnTile| {
            rec.index == t
                || match (class(t), class(rec.index)) {
                    (Some((a, b)), Some((_, rb))) => a != 0 && b == rb,
                    _ => false,
                }
        };
        let found: Vec<SpawnTile> = self
            .level(l)
            .spawn_tiles
            .iter()
            .copied()
            .filter(matches)
            .collect();
        if found.is_empty() {
            return self
                .level(l)
                .spawn_tiles
                .first()
                .copied()
                .unwrap_or_default();
        }
        let r = self.level_mut(l).seed.roll(found.len() as i32) as usize;
        found[r]
    }

    /// The first room with a waypoint flag, streamed; position from its first waypoint
    /// preset object, if any.
    fn waypoint_room(
        &mut self,
        svc: &mut Services<'_>,
        l: LevelIdx,
    ) -> Result<Option<WaypointRoom>, DrlgError> {
        let Some(r) = self
            .level_rooms(l)
            .into_iter()
            .find(|&r| self.room(r).flags & room_flags::ANY_WAYPOINT != 0)
        else {
            return Ok(None);
        };
        self.stream_room(svc, r)?;
        let rect = self.room(r).rect;
        let pos = svc
            .types
            .preset_units(self, r)
            .into_iter()
            .find(|u| u.unit_type == 2 && u.class < 573 && svc.data.is_waypoint_object(u.class))
            .map(|u| (rect.x + u.x / SUBTILES, rect.y + u.y / SUBTILES));
        Ok(Some((r, pos)))
    }

    /// The first room with a warp flag whose warp id ≠ −1.
    fn first_warp_room(
        &self,
        data: &DrlgData,
        l: LevelIdx,
    ) -> Result<Option<DrlgRoomId>, DrlgError> {
        let warp = self.warp_array(data, self.level(l).id)?;
        Ok(self.level_rooms(l).into_iter().find(|&r| {
            let f = self.room(r).flags;
            (0..8).any(|i| f & (room_flags::WARP_0 << i) != 0 && warp[i] != -1)
        }))
    }

    /// Whether a level id is a town or level 120: levels whose rooms are not removed while
    /// any room of the level is still at status 1 or below.
    pub(super) fn no_removal_level(id: u32) -> bool {
        is_town(id) || id == 120
    }
}

/// The acts of a game, each DRLG created on first need.
#[derive(Clone, Debug, Default)]
pub struct Dungeon {
    /// One slot per act.
    pub acts: [Option<Drlg>; 5],
    /// The game's view gap given to each act's DRLG at creation; 0 means
    /// the near gap (a 4:3 game).
    pub view_gap: i32,
}

impl Dungeon {
    /// Sets the game's view gap (0: the near gap, a 4:3 game) for the acts
    /// created from now on and the acts that exist already.
    pub fn set_view_gap(&mut self, gap: i32) {
        self.view_gap = gap;
        let g = if gap == 0 {
            super::room::NEAR_GAP
        } else {
            gap.max(super::room::NEAR_GAP)
        };
        for d in self.acts.iter_mut().flatten() {
            d.view_gap = g;
        }
    }

    /// The act's DRLG, created on first need with the game's init seed and difficulty. The
    /// server's town is the act's town level, or `arena_level` in an arena game.
    pub fn get_or_create(
        &mut self,
        act: u8,
        init_seed: u32,
        difficulty: u8,
        arena_level: Option<u32>,
        data: &DrlgData,
        types: &mut dyn LevelTypes,
    ) -> Result<&mut Drlg, DrlgError> {
        let slot = &mut self.acts[act as usize];
        if slot.is_none() {
            let town = arena_level.unwrap_or(super::TOWN_LEVELS[act as usize]);
            let mut drlg = Drlg::create(act, init_seed, difficulty, town, false, data, types)?;
            if self.view_gap != 0 {
                drlg.view_gap = self.view_gap.max(super::room::NEAR_GAP);
            }
            *slot = Some(drlg);
        }
        Ok(slot.as_mut().expect("created above"))
    }
}
