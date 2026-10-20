//! Preset room grids and unit transfer, the preset tile-fill
//! switches and door preset units.

use super::ds1::unit_type;
use super::map::{style, sub};
use super::{PresetCtx, PresetError, PresetUnit, Presets};
use crate::drlg::level::Drlg;
use crate::drlg::logic::LogicGrids;
use crate::drlg::tiles::{CellGrid, GridPass, RoomGrids};
use crate::drlg::{DrlgRoomId, SUBTILES};

/// Edge bits OR'd on border cells.
const EDGE: u32 = 0x84;
/// lvlprest indexes with the extra zero grid.
const EXTRA_GRID_DEFS: [u32; 2] = [1, 108];
/// Burial Grounds (tombstones).
const TOMBSTONE_LEVEL: u32 = 17;
const MAX_TOMBSTONES: usize = 6;
/// Door objects with the `roll(3)`.
const ROLLED_DOORS: [i32; 2] = [91, 92];

/// What [`Presets::door_unit`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorOutcome {
    /// No door-table row for the level and cell.
    NoRow,
    /// Position outside the room's sub-tile rectangle.
    Outside,
    /// Object 91/92 and `roll(3)` gave 0 (drawn, no unit).
    Rolled0,
    /// A unit was head-inserted on the room's list; the caller sets
    /// flag 0x20 on the door record.
    Placed,
}

impl DoorOutcome {
    /// Whether the door record (when there is one) gets flag 0x20
    /// only after a unit is added and when `roll(3)` gave 0; not when
    /// no table row matches or the position is outside the room. So a
    /// record draws `roll(3)` at most once over all calls.
    pub fn sets_record_flag(self) -> bool {
        matches!(self, DoorOutcome::Placed | DoorOutcome::Rolled0)
    }
}

impl Presets {
    /// The room grids, with the extra-grid and tombstone switches,
    /// [`crate::drlg::LevelTypes::room_grids`]: OR the edge and layer bits
    /// into the shared DS1 layers, cut the room's (w+1)×(h+1) grids, move
    /// the map's units inside the room to the room's list, and (level 17,
    /// once) collect tombstones. No draws.
    ///
    /// With lvlprest `Logicals` ≠ 0, [`RoomGrids::logicals`] carries
    /// wall layer 0's orientation grid, floor layer 0 and wall layer 0 for
    /// the logical-room build; an absent layer
    /// is an empty (zero) grid.
    pub fn room_grids(
        &mut self,
        drlg: &Drlg,
        ctx: &mut PresetCtx<'_>,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, PresetError> {
        let map_id = self.room(room)?.map;
        let rect = drlg.room(room).rect;
        let level_id = drlg.level(drlg.room(room).level).id;
        let (m_rect, d_index, ds1) = {
            let m = self.map(map_id)?;
            (m.rect, m.def, m.ds1.ok_or(PresetError::Ds1NotLoaded)?)
        };
        let def = ctx.data.def(d_index)?;
        let (ox, oy) = (rect.x - m_rect.x, rect.y - m_rect.y);
        let (gw, gh) = ((rect.w + 1) as usize, (rect.h + 1) as usize);
        let stride = (m_rect.w + 1) as usize;
        // Sub-rectangle indexes into the DS1 layers (stride map w + 1).
        let mut idx = Vec::with_capacity(gw * gh);
        for y in 0..gh as i32 {
            for x in 0..gw as i32 {
                let (cx, cy) = (ox + x, oy + y);
                if cx < 0 || cy < 0 {
                    return Err(PresetError::LayerOutOfRange);
                }
                idx.push(cy as usize * stride + cx as usize);
            }
        }
        let border = |k: usize| {
            let (x, y) = (k % gw, k / gw);
            x == 0 || y == 0 || x == gw - 1 || y == gh - 1
        };
        let file = ctx.cache.file_mut(ds1);
        let or_layer = |layer: &mut Vec<u32>, bits: u32, only_border: bool| {
            for (k, &i) in idx.iter().enumerate() {
                if !only_border || border(k) {
                    let c = layer.get_mut(i).ok_or(PresetError::LayerOutOfRange)?;
                    *c |= bits;
                }
            }
            Ok::<(), PresetError>(())
        };
        // Steps 1–4: ORs into the shared buffer.
        for (i, w) in file.walls.iter_mut().enumerate() {
            if i == 0 {
                or_layer(w, EDGE, true)?;
            } else {
                or_layer(w, (i as u32) << 18, false)?;
            }
        }
        for (i, f) in file.floors.iter_mut().enumerate() {
            or_layer(f, (i as u32) << 18, false)?;
        }
        for f in file.floors.iter_mut() {
            or_layer(f, EDGE, true)?;
        }
        or_layer(&mut file.shadow, EDGE, true)?;
        let cut = |layer: &[u32]| -> Result<CellGrid, PresetError> {
            let mut g = CellGrid::new(gw, gh);
            for (k, &i) in idx.iter().enumerate() {
                g.cells[k] = *layer.get(i).ok_or(PresetError::LayerOutOfRange)?;
            }
            Ok(g)
        };
        // Floors, walls (own orientation), shadow,
        // then the extra zero grid of Defs 1 and 108.
        let mut passes = Vec::new();
        for (i, f) in file.floors.iter().enumerate() {
            passes.push(GridPass {
                cells: cut(f)?,
                orientation: None,
                fill_blanks: i == 0 && def.fill_blanks != 0,
            });
        }
        for (w, o) in file.walls.iter().zip(&file.orientations) {
            passes.push(GridPass {
                cells: cut(w)?,
                orientation: Some(cut(o)?),
                fill_blanks: false,
            });
        }
        passes.push(GridPass {
            cells: cut(&file.shadow)?,
            orientation: None,
            fill_blanks: false,
        });
        if EXTRA_GRID_DEFS.contains(&d_index) {
            passes.push(GridPass {
                cells: CellGrid::new(gw, gh),
                orientation: None,
                fill_blanks: false,
            });
        }
        let logicals = if def.logicals != 0 {
            let first = |layers: &[Vec<u32>]| match layers.first() {
                Some(l) => cut(l),
                None => Ok(CellGrid::new(gw, gh)),
            };
            Some(LogicGrids {
                orientation: first(&file.orientations)?,
                floor: first(&file.floors)?,
                wall: first(&file.walls)?,
            })
        } else {
            None
        };
        let kill = def.kill_edge != 0;
        let grids = RoomGrids {
            logicals,
            kill_edge_x: kill && rect.x + rect.w == m_rect.x + m_rect.w,
            kill_edge_y: kill && rect.y + rect.h == m_rect.y + m_rect.h,
            animate: def.animate != 0,
            anim_speed: def.anim_speed,
            passes,
            shadows: Vec::new(),
        };
        // Tombstones (level 17, once per room), from wall grid 0.
        let tombstones = (level_id == TOMBSTONE_LEVEL && self.room(room)?.tombstones.is_none())
            .then(|| {
                let mut out = Vec::new();
                if let Some(GridPass { cells, .. }) = grids.passes.get(file.floors.len()) {
                    if !file.walls.is_empty() {
                        'scan: for y in 0..rect.h {
                            for x in 0..rect.w {
                                let v = cells.get(x, y);
                                if style(v) == 10 && (23..=27).contains(&sub(v)) {
                                    if out.len() == MAX_TOMBSTONES {
                                        break 'scan;
                                    }
                                    out.push((
                                        (rect.x + x) * SUBTILES + 2,
                                        (rect.y + y) * SUBTILES + 2,
                                    ));
                                }
                            }
                        }
                    }
                }
                out
            });
        // Unit transfer.
        let (rx, ry, rw, rh) = (
            rect.x * SUBTILES,
            rect.y * SUBTILES,
            rect.w * SUBTILES,
            rect.h * SUBTILES,
        );
        let m = self.map_mut(map_id)?;
        let mut moved = Vec::new();
        let mut kept = Vec::with_capacity(m.units.len());
        for mut u in std::mem::take(&mut m.units) {
            if rx <= u.x && u.x < rx + rw && ry <= u.y && u.y < ry + rh {
                u.x -= rx;
                u.y -= ry;
                moved.push(u);
            } else {
                kept.push(u);
            }
        }
        m.units = kept;
        let pr = self.rooms.get_mut(&room).expect("checked above");
        for u in moved {
            pr.units.insert(0, u);
        }
        if tombstones.is_some() {
            pr.tombstones = tombstones;
        }
        Ok(grids)
    }

    /// The door cell's preset unit. `wx`, `wy` are the
    /// cell's world tile, `cell` its packed value, `orientation` the
    /// right-door test input (with a record: record type = 9; without
    /// one: the cell orientation = 9). The caller skips a record that
    /// already has flag 0x20 (no lookup, no draw) and sets it when
    /// [`DoorOutcome::sets_record_flag`]. Draws `roll(3)` on the room
    /// seed for objects 91 and 92.
    #[allow(clippy::too_many_arguments)]
    pub fn door_unit(
        &mut self,
        drlg: &mut Drlg,
        ctx: &PresetCtx<'_>,
        room: DrlgRoomId,
        wx: i32,
        wy: i32,
        cell: u32,
        orientation: u32,
    ) -> Result<DoorOutcome, PresetError> {
        let level_id = drlg.level(drlg.room(room).level).id;
        let Some(row) = ctx
            .data
            .tables
            .door(level_id, style(cell), sub(cell), orientation == 9)
            .copied()
        else {
            return Ok(DoorOutcome::NoRow);
        };
        let rect = drlg.room(room).rect;
        let x = (wx - rect.x) * SUBTILES + row.dx;
        let y = (wy - rect.y) * SUBTILES + row.dy;
        if !(0..rect.w * SUBTILES).contains(&x) || !(0..rect.h * SUBTILES).contains(&y) {
            return Ok(DoorOutcome::Outside);
        }
        let (class, mode) = match row.unit_type {
            unit_type::MONSTER => {
                let m = ctx.data.monstats_count as i32;
                (
                    if (0..m).contains(&row.class) {
                        row.class
                    } else {
                        -1
                    },
                    1,
                )
            }
            unit_type::OBJECT => {
                if ROLLED_DOORS.contains(&row.class) && drlg.room_mut(room).seed.roll(3) == 0 {
                    return Ok(DoorOutcome::Rolled0);
                }
                (row.class, 0)
            }
            t => return Err(PresetError::UnsupportedDoorType(t)),
        };
        // A room without a preset map (maze levels) has no unit list to
        // take the unit; the lookup and the draw above still happen.
        if let Some(pr) = self.rooms.get_mut(&room) {
            pr.units.insert(
                0,
                PresetUnit {
                    unit_type: row.unit_type,
                    class,
                    mode,
                    x,
                    y,
                    flags: 0,
                    path: None,
                },
            );
        }
        Ok(DoorOutcome::Placed)
    }
}
