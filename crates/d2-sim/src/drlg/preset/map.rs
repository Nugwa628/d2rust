//! DrlgType 2 levels, preset map allocation, the area build,
//! the unit filter and the first activation of a preset room.

use crate::rng::Seed;

use super::ds1::{unit_type, Ds1File};
use super::{
    MapId, PopEntry, PresetCtx, PresetError, PresetInfo, PresetMap, PresetRoom, PresetUnit, Presets,
};
use crate::drlg::level::Drlg;
use crate::drlg::room::{LinkAt, RoomKind};
use crate::drlg::{room_flags, DrlgRoomId, LevelIdx, TileRect, SUBTILES};

/// Packed-cell main index (style), bits 20–25.
pub(super) fn style(v: u32) -> u32 {
    (v >> 20) & 0x3F
}

/// Packed-cell sub index, bits 8–15.
pub(super) fn sub(v: u32) -> u32 {
    (v >> 8) & 0xFF
}

/// Waypoint scan threshold.
const WAYPOINT_CLASS_LIMIT: i32 = 573;
/// Navi.
const NAVI: i32 = 266;
/// River objects.
const RIVER_SOUND: i32 = 65;
const RIVER_STRIP: [i32; 5] = [40, 41, 41, 41, 42];
/// lvlprest indexes with river objects.
const RIVER_DEFS: [u32; 6] = [1, 3, 26, 27, 28, 300];
/// Floor-cell subs that skip three strip rows.
const RIVER_SKIP_SUBS: [u32; 6] = [0, 4, 8, 16, 29, 39];

impl Presets {
    // ---- DrlgType 2 levels -------------------------------------------

    /// At level allocation: find the row by level id
    /// (none → fatal), then direction := `roll(Files)` on the level seed
    /// if `Files` ≠ 0, else −1 (no draw).
    pub fn init_level(
        &mut self,
        drlg: &mut Drlg,
        ctx: &PresetCtx<'_>,
        level: LevelIdx,
    ) -> Result<(), PresetError> {
        let id = drlg.level(level).id;
        let row = ctx
            .data
            .def_for_level(id)
            .ok_or(PresetError::NoPresetForLevel(id))?;
        let files = ctx.data.def(row)?.files;
        let direction = if files != 0 {
            drlg.level_mut(level).seed.roll(files) as i32
        } else {
            -1
        };
        self.info.insert(
            level,
            PresetInfo {
                map: None,
                direction,
            },
        );
        Ok(())
    }

    /// After the generation seed reset: allocate the
    /// map over the level rectangle (redraws `roll(Files)`), sync the
    /// direction, build the area (no extra flags, multi-room).
    ///
    /// Step 4 (automap callbacks, client DRLG only) needs the room
    /// services: [`Drlg::generate_level_svc`] runs its generic branch
    /// (Vis levels, then every room streamed in list order) after this.
    /// The +0x454 calls are listed in [`Drlg::automap_rooms`] for the
    /// client's automap; the town-art callback +0x488 (levels 40, 103,
    /// 109) runs here and is listed in [`Drlg::automap_towns`].
    pub fn generate(
        &mut self,
        drlg: &mut Drlg,
        ctx: &mut PresetCtx<'_>,
        level: LevelIdx,
    ) -> Result<Option<DrlgRoomId>, PresetError> {
        let id = drlg.level(level).id;
        let row = ctx
            .data
            .def_for_level(id)
            .ok_or(PresetError::NoPresetForLevel(id))?;
        let (def, automap) = {
            let d = ctx.data.def(row)?;
            (d.def, d.automap)
        };
        let rect = drlg.level(level).rect;
        let map = self.alloc_map(drlg, ctx, level, def, rect)?;
        let info = self
            .info
            .get_mut(&level)
            .ok_or(PresetError::NoPresetInfo(level))?;
        let m = self.maps[map.0 as usize].as_mut().expect("just allocated");
        if info.direction == -1 {
            info.direction = m.picked_file;
        } else {
            m.picked_file = info.direction;
        }
        // The info's map field (+0); level reset clears
        // it again.
        info.map = Some(map);
        let picked = m.picked_file;
        let built = self.build_area(drlg, ctx, level, map, 0, false)?;
        // Step 4, town levels: the town-automap callback +0x488 with
        // (level id, picked file, level x + w/2, level y + h/2).
        if drlg.on_client && matches!(id, 40 | 103 | 109) && automap != 0 {
            let centre = (rect.x + rect.w / 2, rect.y + rect.h / 2);
            drlg.automap_towns.push((id, picked as u32, centre));
        }
        Ok(built)
    }

    /// Free the level's map list
    /// (DS1 references, units, pops) and the preset room data of its
    /// maps; map := none, info freed unless `keep`.
    pub fn reset_level(&mut self, ctx: &mut PresetCtx<'_>, level: LevelIdx, keep: bool) {
        let maps = self.level_maps.remove(&level).unwrap_or_default();
        for id in &maps {
            if let Some(m) = self.maps[id.0 as usize].take() {
                if let Some(d) = m.ds1 {
                    ctx.cache.release(d);
                }
            }
        }
        self.rooms.retain(|_, r| !maps.contains(&r.map));
        if keep {
            if let Some(i) = self.info.get_mut(&level) {
                i.map = None;
            }
        } else {
            self.info.remove(&level);
        }
    }

    // ---- map allocation ------------------------------------------------

    /// A new map of lvlprest index `def` at `rect` on
    /// `level`. Picked file := `roll(Files)` on the level seed (the seed
    /// every caller gives; `Files` ≤ 0 → 0, no draw). Size :=
    /// `SizeX`/`SizeY` if both ≠ 0, else the rectangle's. Head-inserted on
    /// the level's map list.
    pub fn alloc_map(
        &mut self,
        drlg: &mut Drlg,
        ctx: &PresetCtx<'_>,
        level: LevelIdx,
        def: u32,
        rect: TileRect,
    ) -> Result<MapId, PresetError> {
        let row = ctx.data.def(def)?;
        let picked_file = drlg.level_mut(level).seed.roll(row.files) as i32;
        let (w, h) = if row.size_x != 0 && row.size_y != 0 {
            (row.size_x as i32, row.size_y as i32)
        } else {
            (rect.w, rect.h)
        };
        let id = MapId(self.maps.len() as u32);
        self.maps.push(Some(PresetMap {
            level,
            def,
            picked_file,
            rect: TileRect::new(rect.x, rect.y, w, h),
            link_grid: None,
            units: Vec::new(),
            hardcoded_pending: true,
            ds1: None,
            pops: Vec::new(),
        }));
        self.level_maps.entry(level).or_default().insert(0, id);
        Ok(id)
    }

    /// The picked file's path (`File1`..`File6`).
    pub fn picked_file_path<'d>(
        &self,
        ctx: &PresetCtx<'d>,
        map: MapId,
    ) -> Result<&'d [u8], PresetError> {
        let m = self.map(map)?;
        let row = ctx.data.def(m.def)?;
        usize::try_from(m.picked_file)
            .ok()
            .and_then(|f| row.file.get(f))
            .map(Vec::as_slice)
            .ok_or(PresetError::BadPickedFile(m.picked_file))
    }

    /// Load the map's DS1 and run the unit filter on `seed`.
    pub(super) fn load_and_filter(
        &mut self,
        ctx: &mut PresetCtx<'_>,
        map: MapId,
        seed: &mut Seed,
    ) -> Result<(), PresetError> {
        let path = self.picked_file_path(ctx, map)?;
        let d = ctx.cache.load(path, ctx.source, ctx.data)?;
        let m = self.maps[map.0 as usize].as_mut().expect("live map");
        m.ds1 = Some(d);
        filter_units(m, ctx.cache.file(d), ctx.data, seed);
        Ok(())
    }

    // ---- area build ----------------------------------------------------

    /// / scan: build the map's area with
    /// extra room flags `flags`, in single-room or multi-room mode.
    /// Returns the last room allocated (`None` for an empty map).
    pub fn build_area(
        &mut self,
        drlg: &mut Drlg,
        ctx: &mut PresetCtx<'_>,
        level: LevelIdx,
        map: MapId,
        flags: u32,
        single: bool,
    ) -> Result<Option<DrlgRoomId>, PresetError> {
        let (def_index, r) = {
            let m = self.map(map)?;
            (m.def, m.rect)
        };
        let def = ctx.data.def(def_index)?.clone();
        // Step 1.
        let mut f = flags;
        if def.outdoors != 0 {
            f |= room_flags::NO_LOS_DRAW;
        }
        // Step 2: cell grid of (w/8 + 1) × (h/8 + 1).
        let gw = (r.w / 8 + 1).max(1) as usize;
        let gh = (r.h / 8 + 1).max(1) as usize;
        // Step 3: warp flags from Vis.
        let level_id = drlg.level(level).id;
        let vis = drlg.vis_array(ctx.drlg, level_id)?;
        for (i, &v) in vis.iter().enumerate() {
            if v != 0 && drlg.warp_id(ctx.drlg, level_id, i)? == -1 {
                f |= room_flags::WARP_0 << i;
            }
        }
        let mut cells = vec![f; gw * gh];
        let cell_index = |x: i32, y: i32| -> usize {
            if single {
                0
            } else {
                (y / 8) as usize * gw + (x / 8) as usize
            }
        };
        // Step 4: no scan, no pops → no DS1 now.
        if def.scan != 0 || def.pops != 0 {
            // Step 5.
            let mut seed = drlg.level(level).seed;
            self.load_and_filter(ctx, map, &mut seed)?;
            drlg.level_mut(level).seed = seed;
            let d = self.map(map)?.ds1.expect("just loaded");
            let file = ctx.cache.file(d);
            if file.width as i32 != r.w || file.height as i32 != r.h {
                return Err(PresetError::SizeMismatch {
                    ds1: (file.width, file.height),
                    map: (r.w, r.h),
                });
            }
            // Step 6–8.
            let mut pops: Vec<PopEntry> = Vec::new();
            let stride = file.stride();
            for (walls, orients) in file.walls.iter().zip(&file.orientations) {
                for y in 0..r.h {
                    for x in 0..r.w {
                        let i = y as usize * stride + x as usize;
                        let (o, v) = (orients[i], walls[i]);
                        if o != 10 && o != 11 {
                            continue;
                        }
                        let (st, sb) = (style(v), sub(v));
                        if def.scan != 0 && st <= 7 && (sb == 0 || sb == 4 || v & 0x8000_0000 != 0)
                        {
                            cells[cell_index(x, y)] |= 1 << (st + 4);
                        }
                        if def.pops != 0 && (8..=29).contains(&st) {
                            // TODO: no capacity check
                            // against `Pops`; the Vec grows.
                            match pops.iter_mut().find(|p| p.group == st) {
                                Some(p) => p.corner2 = (x, y),
                                None => pops.push(PopEntry {
                                    group: st,
                                    sub: sb,
                                    corner1: (x, y),
                                    ..PopEntry::default()
                                }),
                            }
                        }
                        if def.scan != 0 && (30..=33).contains(&st) {
                            let value = match st {
                                30 => sb,
                                31 => sb + 5,
                                32 => 10,
                                _ => 11,
                            };
                            drlg.level_mut(level)
                                .spawn_tiles
                                .push(crate::drlg::SpawnTile {
                                    x: r.x + x,
                                    y: r.y + y,
                                    index: value,
                                });
                        }
                    }
                }
            }
            // Step 8: pops finish.
            for p in &mut pops {
                let (x0, x1) = (p.corner1.0.min(p.corner2.0), p.corner1.0.max(p.corner2.0));
                let (y0, y1) = (p.corner1.1.min(p.corner2.1), p.corner1.1.max(p.corner2.1));
                p.rect = TileRect::new(r.x + x0, r.y + y0, x1 - x0 + 1, y1 - y0 + 1);
                p.group = p.group / 4 - 1;
            }
            // Step 9: waypoints from the file's (unfiltered) list.
            if def.scan != 0 {
                for u in &file.units {
                    if u.unit_type == unit_type::OBJECT
                        && u.class < WAYPOINT_CLASS_LIMIT
                        && u.class >= 0
                        && ctx.drlg.is_waypoint_object(u.class as u32)
                    {
                        let i = if single {
                            0
                        } else {
                            (u.y / SUBTILES / 8) as usize * gw + (u.x / SUBTILES / 8) as usize
                        };
                        // Element cy·(w/8 + 1) + cx, no bound check: cx
                        // past the row width lands in the next row's
                        // cells, as the linear index does.
                        // TODO: a
                        // row outside 0..h/8 reads past the row-offset
                        // array; such a unit is skipped here.
                        if let Some(c) = cells.get_mut(i) {
                            *c |= room_flags::ANY_WAYPOINT;
                        }
                    }
                }
            }
            self.map_mut(map)?.pops = pops;
        }
        // Step 10: rooms.
        let link_grid = self.map(map)?.link_grid.clone();
        let mut last = None;
        let add = |presets: &mut Presets,
                   drlg: &mut Drlg,
                   rect: TileRect,
                   cell: u32,
                   single: bool,
                   link: u32| {
            let id = drlg.alloc_room(level, RoomKind::Preset, rect);
            let room = drlg.room_mut(id);
            room.flags |= cell;
            room.dt1_mask = def.dt1_mask;
            if def.populate == 0 {
                room.flags |= room_flags::NO_POPULATION;
            }
            // A non-zero link would also set bit 0 of the link
            // record's +0x0C; nothing fills the map's link grid (map
            // +0x20 is never set), so the link is always 0 and no link
            // record type is needed.
            presets.rooms.insert(
                id,
                PresetRoom {
                    def: def_index,
                    map,
                    single,
                    link,
                    units: Vec::new(),
                    tombstones: None,
                },
            );
            // Head insert and count, so the
            // level list holds the rooms in reverse creation order.
            drlg.link_room(id, LinkAt::Head);
            id
        };
        if single {
            last = Some(add(self, drlg, r, cells[0], true, 0));
        } else {
            let mut y = 0;
            while y < r.h {
                let mut x = 0;
                while x < r.w {
                    let rect = TileRect::new(r.x + x, r.y + y, (r.w - x).min(8), (r.h - y).min(8));
                    let i = cell_index(x, y);
                    let link = link_grid
                        .as_ref()
                        .and_then(|g| g.get(i).copied())
                        .unwrap_or(0);
                    last = Some(add(self, drlg, rect, cells[i], false, link));
                    x += 8;
                }
                y += 8;
            }
        }
        Ok(last)
    }

    // ---- first activation ----------------------------------------------

    /// [`crate::drlg::LevelTypes::add_preset_units`]:
    /// lazy DS1 load and filter on this room's seed, hardcoded units (navi,
    /// river objects), then room flag `0x2000000`.
    pub fn add_preset_units(
        &mut self,
        drlg: &mut Drlg,
        ctx: &mut PresetCtx<'_>,
        room: DrlgRoomId,
    ) -> Result<(), PresetError> {
        let map = self.room(room)?.map;
        // Step 1.
        if self.map(map)?.ds1.is_none() {
            let mut seed = drlg.room(room).seed;
            self.load_and_filter(ctx, map, &mut seed)?;
            drlg.room_mut(room).seed = seed;
        }
        // Step 2.
        let level_id = drlg.level(drlg.room(room).level).id;
        let room_flags_now = drlg.room(room).flags;
        let m = self.maps[map.0 as usize].as_mut().expect("live map");
        if m.hardcoded_pending {
            m.hardcoded_pending = false;
            let d = m.def;
            if (4..=7).contains(&d) && level_id == 2 && m.picked_file == 3 {
                let class = if ctx.data.monstats_count <= NAVI as u32 {
                    -1
                } else {
                    NAVI
                };
                let (x, y) = (
                    (m.rect.x + m.rect.w / 2) * SUBTILES,
                    (m.rect.y + m.rect.h / 2) * SUBTILES,
                );
                m.units.insert(
                    0,
                    PresetUnit {
                        unit_type: unit_type::MONSTER,
                        class,
                        mode: 1,
                        x,
                        y,
                        flags: 0,
                        path: None,
                    },
                );
            } else if RIVER_DEFS.contains(&d) && room_flags_now & room_flags::AUTOMAP_REVEAL != 0 {
                let d1 = m.ds1.ok_or(PresetError::Ds1NotLoaded)?;
                river_objects(m, ctx.cache.file(d1))?;
            }
        }
        // Step 3.
        drlg.room_mut(room).flags |= room_flags::PRESET_UNITS_ADDED;
        Ok(())
    }
}

// ---- unit filter -------------------------------------------------------

/// Walk the file's list (reverse file order); kept
/// units are copied with (map x·5, map y·5) added (path points too) and
/// head-inserted on the map list.
fn filter_units(m: &mut PresetMap, file: &Ds1File, pd: &super::PresetData, seed: &mut Seed) {
    let (sx, sy) = (m.rect.x * SUBTILES, m.rect.y * SUBTILES);
    let ms = (pd.monstats_count + pd.superuniques_count) as i32;
    for u in &file.units {
        let keep = match (u.unit_type, u.class) {
            (unit_type::MONSTER, 204 | 205 | 371 | 372) => seed.step().is_multiple_of(3),
            (unit_type::MONSTER, c) if c == ms + 33 => seed.step() & 3 != 0,
            (unit_type::MONSTER, c) if c == ms + 34 => seed.step() & 1 != 0,
            (unit_type::MONSTER, c) if c == ms + 35 => seed.step() & 3 == 0,
            (unit_type::OBJECT, 196 | 261) => seed.step() & 1 == 0,
            (unit_type::OBJECT, 581) => seed.step() & 3 != 0,
            _ => true,
        };
        if keep {
            let mut c = u.clone();
            c.x += sx;
            c.y += sy;
            if let Some(p) = c.path.as_mut() {
                for pt in p {
                    pt.x += sx;
                    pt.y += sy;
                }
            }
            m.units.insert(0, c);
        }
    }
}

// ---- river objects -----------------------------------------------------

/// River sound and strip objects from the DS1's floor
/// layer 0 (stride map w + 1), head-inserted on the map list.
fn river_objects(m: &mut PresetMap, file: &Ds1File) -> Result<(), PresetError> {
    let floor = file.floors.first().ok_or(PresetError::LayerOutOfRange)?;
    let r = m.rect;
    let stride = (r.w + 1) as usize;
    let g = |c: i32, row: i32| -> Result<u32, PresetError> {
        floor
            .get(row as usize * stride + c as usize)
            .copied()
            .ok_or(PresetError::LayerOutOfRange)
    };
    let object = |class: i32, x: i32, y: i32| PresetUnit {
        unit_type: unit_type::OBJECT,
        class,
        mode: 0,
        x,
        y,
        flags: 1,
        path: None,
    };
    let mut out: Vec<PresetUnit> = Vec::new();
    let sounds = |out: &mut Vec<PresetUnit>, cx: i32| {
        let end = (r.y + file.height as i32) * SUBTILES;
        let mut sy = r.y * SUBTILES;
        while sy < end {
            out.push(object(RIVER_SOUND, cx * SUBTILES, sy));
            sy += 40;
        }
    };
    let strip = |out: &mut Vec<PresetUnit>, c: i32| -> Result<(), PresetError> {
        let mut row = 0;
        while row < r.h {
            let x0 = (r.x + c) * SUBTILES - 5;
            let y = (r.y + row) * SUBTILES;
            for (k, &class) in RIVER_STRIP.iter().enumerate() {
                out.push(object(class, x0 + 5 * k as i32, y));
            }
            let v = g(c.max(0), row)?;
            if style(v) == 4 && RIVER_SKIP_SUBS.contains(&sub(v)) {
                row += 3;
            }
            row += 1;
        }
        Ok(())
    };
    if m.def == 27 {
        sounds(&mut out, r.x);
        strip(&mut out, -1)?;
    } else {
        let mut c = 0;
        while c < r.w {
            let v = g(c, 0)?;
            if style(v) == 2 && sub(v) == 24 {
                sounds(&mut out, r.x + c + 1);
                strip(&mut out, c)?;
                c += 4;
            }
            c += 1;
        }
    }
    for u in out {
        m.units.insert(0, u);
    }
    Ok(())
}
