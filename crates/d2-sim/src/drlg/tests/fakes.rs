//! Small fakes: a leveldefs view, a scripted [`LevelTypes`], a DT1
//! source and the real `UnitLists` as the act room list.

use std::collections::BTreeMap;

use crate::drlg::room::LinkAt;
use crate::drlg::tiles::cell;
use crate::drlg::*;
use crate::units::UnitLists;

/// Level ids 0..150, all DRLG type 0 (no generation) unless set.
pub fn data() -> DrlgData {
    let mut d = DrlgData {
        levels: vec![LevelDef::default(); 150],
        ..DrlgData::default()
    };
    for l in &mut d.levels {
        l.warp = [-1; 8];
    }
    // Level type 1: one file.
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    d.lvltypes = vec![vec![Vec::new(); 32], files];
    d
}

/// Makes `id` a generated level of DRLG type `ty` (level type 1).
pub fn gen_level(d: &mut DrlgData, id: u32, ty: u32) {
    d.levels[id as usize].drlg_type = ty;
    d.levels[id as usize].level_type = 1;
}

/// One scripted room: rect, kind, flags.
#[derive(Clone, Debug)]
pub struct RoomSpec {
    pub rect: TileRect,
    pub kind: RoomKind,
    pub flags: u32,
}

pub fn preset(x: i32, y: i32, w: i32, h: i32) -> RoomSpec {
    RoomSpec {
        rect: TileRect::new(x, y, w, h),
        kind: RoomKind::Preset,
        flags: 0,
    }
}

/// Scripted level types: rooms per level id, grids per (level id, room
/// index), act levels to allocate in order.
#[derive(Default)]
pub struct FakeTypes {
    pub rooms: BTreeMap<u32, Vec<RoomSpec>>,
    pub grids: BTreeMap<(u32, usize), RoomGrids>,
    /// Default grid builder (room rect → grids) when no entry exists.
    pub default_grid: Option<fn(TileRect) -> RoomGrids>,
    pub act_levels: Vec<u32>,
    pub generated: Vec<u32>,
    pub inits: Vec<u32>,
    pub preset_units: BTreeMap<u32, Vec<PresetUnit>>,
    pub preset_units_added: Vec<DrlgRoomId>,
    pub resets: Vec<u32>,
    pub spawn_tiles: BTreeMap<u32, Vec<SpawnTile>>,
    /// (wx, wy) of door and warp unit hooks.
    pub door_units: Vec<(i32, i32)>,
    pub warp_units: Vec<(i32, i32)>,
    /// What the door unit hook answers (record flag 0x20).
    pub door_flag: bool,
    /// Levels whose generation streams their rooms on a client copy.
    pub automap_levels: Vec<u32>,
}

impl LevelTypes for FakeTypes {
    fn automap_streams(&self, drlg: &Drlg, level: LevelIdx) -> bool {
        self.automap_levels.contains(&drlg.level(level).id)
    }

    fn create_act_levels(&mut self, drlg: &mut Drlg, data: &DrlgData) -> Result<(), DrlgError> {
        for id in self.act_levels.clone() {
            drlg.get_or_alloc_level(data, self, id)?;
        }
        Ok(())
    }

    fn init_level(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        self.inits.push(drlg.level(level).id);
        Ok(())
    }

    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        let id = drlg.level(level).id;
        self.generated.push(id);
        for spec in self.rooms.get(&id).cloned().unwrap_or_default() {
            let r = drlg.alloc_room(level, spec.kind, spec.rect);
            drlg.room_mut(r).flags |= spec.flags;
            drlg.room_mut(r).dt1_mask = 1;
            drlg.link_room(r, LinkAt::Tail);
        }
        if let Some(t) = self.spawn_tiles.get(&id) {
            drlg.level_mut(level).spawn_tiles = t.clone();
        }
        Ok(())
    }

    fn reset_level(&mut self, drlg: &mut Drlg, level: LevelIdx) {
        self.resets.push(drlg.level(level).id);
    }

    fn add_preset_units(&mut self, _: &mut Drlg, room: DrlgRoomId) -> Result<(), DrlgError> {
        self.preset_units_added.push(room);
        Ok(())
    }

    fn preset_units(&self, drlg: &Drlg, room: DrlgRoomId) -> Vec<PresetUnit> {
        let l = drlg.level(drlg.room(room).level).id;
        self.preset_units.get(&l).cloned().unwrap_or_default()
    }

    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        let l = drlg.room(room).level;
        let idx = drlg.level_rooms(l).iter().position(|&r| r == room).unwrap();
        let key = (drlg.level(l).id, idx);
        if let Some(g) = self.grids.get(&key) {
            return Ok(g.clone());
        }
        Ok(self
            .default_grid
            .map(|f| f(drlg.room(room).rect))
            .unwrap_or_default())
    }

    fn door_unit(
        &mut self,
        _: &mut Drlg,
        _: &DrlgData,
        _: DrlgRoomId,
        wx: i32,
        wy: i32,
        _: u32,
        _: u32,
    ) -> bool {
        self.door_units.push((wx, wy));
        self.door_flag
    }

    #[allow(clippy::too_many_arguments)]
    fn warp_unit(
        &mut self,
        _: &mut Drlg,
        _: &DrlgData,
        _: DrlgRoomId,
        wx: i32,
        wy: i32,
        _: u32,
        _: u32,
    ) -> Result<bool, DrlgError> {
        self.warp_units.push((wx, wy));
        Ok(true)
    }
}

/// One floor pass, every cell a floor with key (0, 0), edge row and
/// column linked.
pub fn floor_grid(r: TileRect) -> RoomGrids {
    let (w, h) = (r.w as usize + 1, r.h as usize + 1);
    let mut g = CellGrid::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            g.set(x, y, cell::FLOOR | if edge { cell::LINKED } else { 0 });
        }
    }
    RoomGrids {
        passes: vec![GridPass {
            cells: g,
            orientation: None,
            fill_blanks: false,
        }],
        ..RoomGrids::default()
    }
}

pub fn tile(o: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation: o,
        main,
        sub,
        rarity,
        material: 0,
        subtile_flags: [0; 25],
        roof_height: 0,
        height: 0,
        light_direction: 0,
    }
}

/// DT1 files by path.
#[derive(Default)]
pub struct FakeTiles(pub BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for FakeTiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(|v| v.as_slice())
    }
}

/// `floor.dt1`: four (0, 0, 0) floors of rarity 1 (sub-tile flag byte 0
/// = 1, the bottom-left sub-tile), walls and shadows of key (1, 0) with
/// two tiles each, three animated (0, 2, 0) frames; the fixed library
/// files (Blank.dt1: (0, 30, 0) and (0, 30, 1), flags 0x20 everywhere;
/// Warp.dt1 a (10, 0, 0) tile).
pub fn tiles() -> FakeTiles {
    let mut t = FakeTiles::default();
    let mut floor: Vec<TileInfo> = (0..4)
        .map(|_| {
            let mut x = tile(0, 0, 0, 1);
            x.subtile_flags[0] = 1;
            x
        })
        .collect();
    for o in [1, 2, 3, 4, 13] {
        floor.push(tile(o, 1, 0, 1));
        floor.push(tile(o, 1, 0, 1));
    }
    for k in 0..3 {
        let mut x = tile(0, 2, 0, k);
        x.material = 0x100;
        floor.push(x);
    }
    t.0.insert(b"floor.dt1".to_vec(), floor);
    let blank = |sub| {
        let mut x = tile(0, 30, sub, 0);
        x.subtile_flags = [0x20; 25];
        x
    };
    t.0.insert(tiles::FIXED_LIBRARY[0].to_vec(), vec![blank(0), blank(1)]);
    t.0.insert(tiles::FIXED_LIBRARY[1].to_vec(), vec![]);
    t.0.insert(tiles::FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    t
}

/// Everything a test needs.
pub struct World {
    pub data: DrlgData,
    pub types: FakeTypes,
    pub tiles: FakeTiles,
    pub lists: UnitLists,
}

impl World {
    pub fn new(data: DrlgData, types: FakeTypes) -> Self {
        Self {
            data,
            types,
            tiles: tiles(),
            lists: UnitLists::new(),
        }
    }

    pub fn svc(&mut self) -> Services<'_> {
        Services {
            data: &self.data,
            tiles: &self.tiles,
            types: &mut self.types,
            rooms: &mut self.lists,
        }
    }

    /// A server DRLG of act 0 with no town.
    pub fn drlg(&mut self, init_seed: u32) -> Drlg {
        Drlg::create(0, init_seed, 0, 0, false, &self.data, &mut self.types).unwrap()
    }
}

/// A 3×3 block of 8×8 preset rooms (row-major) in level `id` at (0, 0).
pub fn grid3x3(types: &mut FakeTypes, id: u32) {
    let mut v = Vec::new();
    for k in 0..9 {
        v.push(preset(8 * (k % 3), 8 * (k / 3), 8, 8));
    }
    types.rooms.insert(id, v);
}
