//! Outdoor level generation entry point, the step that turns grid cells into rooms,
//! and the per-room grids of an outdoor room.
//!
//! `Outdoor::generate` builds the vertex polygon, runs the act-specific generator, then
//! `cells_to_rooms` allocates one 8x8-tile room per filled cell. `room_grids` later
//! fills a room's floor and wall grids, using the path floor and tile substitutions.

use super::super::data::DrlgData;
use super::super::level::Drlg;
use super::super::room::{LinkAt, RoomKind};
use super::super::tiles::{cell as tcell, CellGrid, GridPass, RoomGrids};
use super::super::{room_flags, DrlgRoomId, LevelIdx, TileRect};
use super::grid::{cell, Gen, Grid};
use super::path_floor::path_floor;
use super::tilesub::{pick_sub_themes, room_substitution, RoomSub};
use super::{
    dispatch_act, Outdoor, OutdoorData, OutdoorError, OutdoorLevel, OutdoorPresets, OutdoorRoom,
    SubFiles,
};
use crate::rng::Seed;

/// Room tile size of an outdoor room.
pub const ROOM_TILES: i32 = 8;

/// DT1 mask a room starts with, by level type.
pub fn dt1_mask(level_type: u32) -> u32 {
    match level_type {
        2 => 0x44103,
        16 | 22 | 27 | 28 => 0x1,
        21 => 0x4,
        30 | 31 => 0x11,
        _ => 0,
    }
}

/// Floor flags by level type (and level id for type 31), OR'd into floor cells that have
/// none of the bits in `0x3F0FF80`.
pub fn floor_flags(level_type: u32, id: u32) -> u32 {
    match level_type {
        16 => 0x100,
        21 => 0x12_0000,
        22 => 0x10_0000,
        27 => 0xA0_0000,
        28 => 0x160_0000,
        31 if id == 117 => 0x60_0000,
        _ => 0,
    }
}

impl Outdoor {
    /// Generates one outdoor level: polygon, act-specific layout, then rooms.
    ///
    /// The caller has already reset the level seed.
    pub fn generate(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        od: &OutdoorData,
        subs: &dyn SubFiles,
        presets: &mut dyn OutdoorPresets,
        level: LevelIdx,
    ) -> Result<(), OutdoorError> {
        let mut info = self.levels.remove(&level).unwrap_or_default();
        let r = self.generate_with(drlg, data, od, subs, presets, level, &mut info);
        self.levels.insert(level, info);
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn generate_with(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        od: &OutdoorData,
        subs: &dyn SubFiles,
        presets: &mut dyn OutdoorPresets,
        level: LevelIdx,
        info: &mut OutdoorLevel,
    ) -> Result<(), OutdoorError> {
        let rect = drlg.level(level).rect;
        let id = drlg.level(level).id;
        // Reset the four working grids to the level's size in 8-tile cells.
        let (gw, gh) = (rect.w / 8, rect.h / 8);
        info.grids = [
            Grid::new(gw, gh),
            Grid::new(gw, gh),
            Grid::new(gw, gh),
            Grid::new(gw, gh),
        ];
        info.vertices.clear();
        info.path_ends.clear();
        info.paths.clear();
        info.build_list.clear();
        info.sub_hits.clear();
        let mut g = Gen {
            drlg,
            data,
            od,
            subs,
            level,
            id,
            rect,
            info,
        };
        // Build the vertex polygon from the level rect and neighbour entries.
        g.polygon()?;
        // Run the generator for the level's act.
        match dispatch_act(id) {
            0 => g.act1()?,
            1 => g.act2()?,
            2 => g.act3()?,
            3 => g.act4()?,
            _ => g.act5()?,
        }
        // Allocate rooms for the finished cells.
        self.cells_to_rooms(&mut g, presets)
    }

    /// Turns the finished grid cells into rooms.
    ///
    /// A preset cell builds its preset; any other cell not marked blank becomes an
    /// outdoor room.
    pub fn cells_to_rooms(
        &mut self,
        g: &mut Gen<'_>,
        presets: &mut dyn OutdoorPresets,
    ) -> Result<(), OutdoorError> {
        let lt = g.drlg.level(g.level).level_type;
        let mask = dt1_mask(lt);
        let sd = g.od.sub_defs(g.id);
        for y in 0..g.gh() {
            for x in 0..g.gw() {
                let (ox, oy) = (g.rect.x + 8 * x, g.rect.y + 8 * y);
                let f1 = g.g(1, x, y);
                let f2 = g.g(2, x, y);
                if f2 & cell::PRESET != 0 {
                    let p = g.g(0, x, y);
                    if p != 0 {
                        presets.build_preset_cell(
                            g.drlg,
                            g.data,
                            g.level,
                            p,
                            ox,
                            oy,
                            (f2 >> 16) & 0xF,
                            f1,
                        )?;
                    }
                } else if f2 & cell::BLANK == 0 {
                    // A plain outdoor cell becomes an 8x8-tile room.
                    let id = g.drlg.alloc_room(
                        g.level,
                        RoomKind::Outdoor,
                        TileRect::new(ox, oy, ROOM_TILES, ROOM_TILES),
                    );
                    // The room is linked at the head of the level's list (newest
                    // first).
                    g.drlg.link_room(id, LinkAt::Head);
                    let room = g.drlg.room_mut(id);
                    room.flags |= f1 | room_flags::NO_LOS_DRAW;
                    room.dt1_mask = mask;
                    let (picked, dt1) =
                        pick_sub_themes(g.od, &mut room.seed, sd.sub_type, sd.sub_theme)?;
                    room.dt1_mask |= dt1;
                    self.rooms.insert(
                        id,
                        OutdoorRoom {
                            flags: f2,
                            flags_ex: g.g(3, x, y),
                            sub_type: sd.sub_type,
                            sub_theme: sd.sub_theme,
                            picked,
                            ..OutdoorRoom::default()
                        },
                    );
                }
            }
        }
        Ok(())
    }

    /// Builds the wall and floor grids of one outdoor room.
    ///
    /// The caller has already reset the room seed.
    pub fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        od: &OutdoorData,
        subs: &dyn SubFiles,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, OutdoorError> {
        let r = drlg.room(room);
        let (rect, flags, level) = (r.rect, r.flags, r.level);
        let (id, lt) = (drlg.level(level).id, drlg.level(level).level_type);
        let sd = od.sub_defs(id);
        let n = (ROOM_TILES + 1) as usize;
        let act1 = drlg.act == 0;
        let paths = self.levels.get(&level).map(|l| &l.paths);
        let data = self.rooms.entry(room).or_default();
        data.tile_type = CellGrid::new(n, n);
        data.wall = CellGrid::new(n, n);
        data.floor = CellGrid::new(n, n);
        data.roof_count = 0;
        data.shadows.clear();
        data.shadow_tiles.clear();
        data.units.clear();
        for y in 0..ROOM_TILES as usize {
            for x in 0..ROOM_TILES as usize {
                data.floor.set(x, y, 0x40002);
            }
        }
        // Act I: path floor, laid before the substitutions (its floor cells fail the
        // substitutions' fixed-tile test).
        if act1 {
            if let Some(paths) = paths {
                path_floor(paths, rect, &mut data.floor);
            }
        }
        let (sub_type, sub_theme, picked) = (data.sub_type, data.sub_theme, data.picked);
        // The room seed is lent to the substitutions; the shadow tile choices draw
        // from it too.
        let mut room_seed = drlg.room(room).seed;
        let seed = &mut room_seed;
        let lib: &Drlg = drlg;
        let pick =
            |s: &mut Seed, v: u32| lib.choose_tile_with(room, 13, tcell::main(v), tcell::sub(v), s);
        let mut rs = RoomSub {
            w: ROOM_TILES,
            h: ROOM_TILES,
            tile_x: rect.x,
            tile_y: rect.y,
            room: data,
            pick: Some(&pick),
        };
        let wp = (flags >> 16) & 3;
        if wp != 0 {
            room_substitution(od, subs, seed, &mut rs, sd.sub_waypoint, 0, wp)?;
        }
        let sh = (flags >> 12) & 0xF;
        if sh != 0 {
            room_substitution(od, subs, seed, &mut rs, sd.sub_shrine, 0, sh)?;
        }
        room_substitution(od, subs, seed, &mut rs, sub_type, sub_theme, picked)?;
        drlg.room_mut(room).seed = room_seed;
        let ff = floor_flags(lt, id);
        for c in &mut data.floor.cells {
            if *c & 0x3F0_FF80 == 0 {
                *c |= ff;
            }
        }
        // Mark the grid edges: all four border rows and columns of the (w+1)×(h+1)
        // grids get bit 0x4.
        for g in [&mut data.wall, &mut data.floor] {
            for y in 0..n {
                for x in 0..n {
                    if x == 0 || y == 0 || x == n - 1 || y == n - 1 {
                        let v = g.get(x as i32, y as i32);
                        g.set(x, y, v | 0x4);
                    }
                }
            }
        }
        // Shadow records made during the substitutions. Roof growth stays in
        // `OutdoorRoom` and is not part of the returned grids.
        let shadows = data
            .shadows
            .iter()
            .zip(&data.shadow_tiles)
            .map(|(&(x, y, v), &t)| (x, y, v, t))
            .collect();
        Ok(RoomGrids {
            passes: vec![
                GridPass {
                    cells: data.wall.clone(),
                    orientation: Some(data.tile_type.clone()),
                    fill_blanks: false,
                },
                GridPass {
                    cells: data.floor.clone(),
                    orientation: None,
                    fill_blanks: false,
                },
            ],
            shadows,
            ..RoomGrids::default()
        })
    }
}
