//! Integration tests: the real DRLG, level-type, population, init, unit
//! and AI modules run together through the world-generation adapters on
//! synthetic tables and synthetic DS1 files. The act DRLG is created
//! through the dispatcher (the Act I placement of a known outdoor
//! layout), so every level of the act is allocated by the real level
//! types. Only the seams without a provider ([`WorldPending`],
//! [`Pending`]) are faked: positions, coordinate lists, a call log.

mod e2e;
mod events;
mod init;
mod levels;
mod maze;
mod outdoor;
mod population;
mod warp_tiles;

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Difficultylevels, Levels, Monlvl, Monstats, Monstats2};

use super::*;
use crate::drlg::maze::{Maze, MazeData, MazeRow, Specials};
use crate::drlg::outdoor::{OutdoorData, PresetDef as OutdoorPreset, SubDefs, SubFileMap};
use crate::drlg::preset::{
    Ds1Input, Ds1ObjectInput, Ds1Source, PresetData, PresetDef, PresetTables,
};
use crate::drlg::tiles::{cell, FIXED_LIBRARY};
use crate::drlg::{
    Drlg, DrlgData, DrlgError, DrlgRoomId, LevelDef, LevelIdx, Services, TileInfo, TileSource,
};
use crate::game::Game;
use crate::monsters::init::MonstatsExtra;
use crate::monsters::population::{CoordRect, PopTables};
use crate::skills::fake::{blank, combat_tables, monster_rec, skill_rec, skill_tables};
use crate::units::hooks::{MonsterInfo, UnitData};
use crate::units::RoomId;
use crate::wiring::action::{ActionHooks, ActionTables, DrlgWorld};

/// The DRLG init seed of a known Act I placement:
/// `dwStartSeed` 4014346869.
pub const INIT: u32 = 644_409_375;
pub const START: u32 = 4_014_346_869;
/// The game seed of the fixture.
pub const GAME_SEED: u32 = 1234;
/// Monastery Gate: a preset level of the Act I chain (40 × 18).
pub const GATE: u32 = 26;
/// A preset level outside the Act I chain (40 × 18 at (8000, 8000), no
/// neighbours): the level the population tests build (level 30's id;
/// streaming a chain level would generate its outdoor neighbours).
pub const ISLE: u32 = 30;
/// Den of Evil: a maze level (level type 3).
pub const DEN: u32 = 8;
/// lvlprest rows of the Monastery Gate and of [`ISLE`].
pub const GATE_DEF: u32 = 1101;
pub const ISLE_DEF: u32 = 1104;
/// lvlprest row of the Rogue Encampment (Files 0) and Outer Cloister
/// (Files 3).
pub const TOWN_DEF: u32 = 1100;
pub const CLOISTER_DEF: u32 = 1102;
/// lvlprest row used as an outdoor preset cell (16 × 16, Files 2).
pub const CELL_DEF: u32 = 1103;
/// The [`ISLE`] DS1's monster: DS1 sub-tile (12, 10), class 0.
pub const ISLE_MONSTER: (u32, u32) = (12, 10);

// ---- seams without a provider -----------------------------------------------

/// Positions (allocation places the unit) and a log of the pending calls
/// that matter. The DRLG data population reads come from the real act
/// DRLG.
#[derive(Default)]
pub struct TestPending {
    pub pos: BTreeMap<UnitId, (i32, i32)>,
    pub log: Vec<String>,
}

impl Pending for TestPending {
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or((0, 0))
    }
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {
        self.pos.insert(unit, (x, y));
    }
    fn class_has_mode(&self, _: i32, _: u8) -> bool {
        true
    }
    fn monster_quest_chain(&mut self, unit: UnitId, chain: u32) {
        self.log.push(format!("chain {} {chain}", unit.0));
    }
}

impl WorldPending for TestPending {
    fn set_alignment(&mut self, unit: UnitId, align: u8) {
        self.log.push(format!("align {} {align}", unit.0));
    }
    fn preset_created(&mut self, unit: UnitId, p: &crate::monsters::population::PresetUnit) {
        self.log.push(format!(
            "preset {} class {} at {},{}",
            unit.0, p.class, p.x, p.y
        ));
    }
}

// ---- DS1 and DT1 sources ---------------------------------------------------------

/// Parsed DS1 files by lvlprest path.
#[derive(Default)]
pub struct Ds1s(pub BTreeMap<Vec<u8>, Ds1Input>);

impl Ds1Source for Ds1s {
    fn ds1(&self, path: &[u8]) -> Option<&Ds1Input> {
        self.0.get(path)
    }
}

/// A v18 DS1 of `w × h` tiles: one floor layer of plain floors, no
/// walls, an empty shadow layer, monster objects (id = class) at DS1
/// sub-tile positions.
pub fn ds1(w: u32, h: u32, monsters: &[(u32, u32, u32)]) -> Ds1Input {
    let cells = ((w + 1) * (h + 1)) as usize;
    Ds1Input {
        version: 18,
        width: w,
        height: h,
        act: 0,
        tag_type: 0,
        walls: Vec::new(),
        orientations: Vec::new(),
        floors: vec![vec![cell::FLOOR; cells]],
        shadow: vec![0; cells],
        objects: monsters
            .iter()
            .map(|&(id, x, y)| Ds1ObjectInput {
                kind: 1,
                id,
                x,
                y,
                flags: 0,
            })
            .collect(),
        paths: Vec::new(),
    }
}

fn tile(o: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
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

/// DT1 files by path: open floors (0, 0, 0), the fixed library files.
struct Tiles(BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for Tiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

fn tiles() -> Tiles {
    let mut t = BTreeMap::new();
    t.insert(b"floor.dt1".to_vec(), vec![tile(0, 0, 0, 1); 4]);
    let blank_tile = |sub| {
        let mut x = tile(0, 30, sub, 0);
        x.subtile_flags = [0x20; 25];
        x
    };
    t.insert(
        FIXED_LIBRARY[0].to_vec(),
        vec![blank_tile(0), blank_tile(1)],
    );
    t.insert(FIXED_LIBRARY[1].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    Tiles(t)
}

// ---- tables --------------------------------------------------------------------------

/// The Act I tables of that known placement (sizes,
/// offsets, Vis), plus the maze level 8 and level type 1 with one DT1.
pub fn drlg_data() -> DrlgData {
    let mut d = DrlgData {
        levels: vec![LevelDef::default(); 150],
        ..DrlgData::default()
    };
    for l in &mut d.levels {
        l.warp = [-1; 8];
        l.level_type = 1;
    }
    let set = |d: &mut DrlgData, id: usize, ty: u32, size: (i32, i32), off: (i32, i32)| {
        d.levels[id].drlg_type = ty;
        d.levels[id].size = [size; 3];
        d.levels[id].offset = off;
    };
    set(&mut d, 1, 2, (56, 40), (0, 0));
    set(&mut d, 2, 3, (56, 96), (0, 0));
    set(&mut d, 3, 3, (80, 80), (0, 0));
    set(&mut d, 4, 3, (80, 80), (1000, 1000));
    set(&mut d, 5, 3, (80, 80), (0, 0));
    set(&mut d, 6, 3, (80, 80), (0, 0));
    set(&mut d, 7, 3, (80, 80), (0, 0));
    set(&mut d, 17, 3, (40, 48), (0, 0));
    set(&mut d, 26, 2, (40, 18), (3000, 1000));
    set(&mut d, 27, 2, (40, 40), (0, 0));
    set(&mut d, 39, 3, (64, 64), (5000, 1148));
    set(&mut d, ISLE as usize, 2, (40, 18), (8000, 8000));
    set(&mut d, DEN as usize, 1, (200, 200), (1500, 1000));
    d.levels[DEN as usize].level_type = 3;
    d.levels[2].vis = [1, 3, 0, 0, 0, 0, 0, 0];
    d.levels[3].vis = [2, 4, 17, 0, 0, 0, 0, 0];
    d.levels[1].vis = [2, 0, 0, 0, 0, 0, 0, 0];
    d.levels[4].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    d.levels[17].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    d.lvltypes = vec![
        vec![Vec::new(); 32],
        files.clone(),
        vec![Vec::new(); 32],
        files,
    ];
    d
}

/// lvlprest: rows 0..1199 with `Files` 1 and DT1 mask 1 (maze defs);
/// the preset levels' rows (`LevelId` 1, 26, 27) and an outdoor cell row.
pub fn preset_data() -> PresetData {
    let mut defs: Vec<PresetDef> = (0..1200)
        .map(|i| PresetDef {
            def: i,
            files: 1,
            dt1_mask: 1,
            populate: 1,
            file: Default::default(),
            ..PresetDef::default()
        })
        .collect();
    for d in &mut defs {
        d.file[0] = format!("def{}.ds1", d.def).into_bytes();
    }
    let row = |defs: &mut Vec<PresetDef>, i: u32, level: u32, files: i32| {
        let d = &mut defs[i as usize];
        d.level_id = level;
        d.files = files;
    };
    row(&mut defs, TOWN_DEF, 1, 0);
    row(&mut defs, GATE_DEF, GATE, 1);
    row(&mut defs, CLOISTER_DEF, 27, 3);
    row(&mut defs, ISLE_DEF, ISLE, 1);
    row(&mut defs, CELL_DEF, 0, 2);
    defs[CELL_DEF as usize].size_x = 16;
    defs[CELL_DEF as usize].size_y = 16;
    PresetData {
        defs,
        monpreset_acts: Default::default(),
        monpreset: Vec::new(),
        monstats_count: 1,
        superuniques_count: 0,
        hdm_item: -1,
        tables: PresetTables::spec().expect("preset-tables.tsv"),
    }
}

/// Outdoor view: no sub themes; lvlprest sizes and `Files` as
/// [`preset_data`].
pub fn outdoor_data(pd: &PresetData) -> OutdoorData {
    OutdoorData {
        levels: vec![
            SubDefs {
                sub_type: -1,
                sub_theme: -1,
                sub_waypoint: -1,
                sub_shrine: -1,
            };
            150
        ],
        presets: pd
            .defs
            .iter()
            .map(|d| OutdoorPreset {
                size_x: d.size_x as i32,
                size_y: d.size_y as i32,
                files: d.files,
            })
            .collect(),
        subs: Vec::new(),
    }
}

/// The Den of Evil maze row (1 room, cells 24).
pub fn maze_data() -> MazeData {
    MazeData {
        rows: vec![MazeRow {
            level: DEN,
            rooms: [1; 3],
            size_x: 24,
            size_y: 24,
            merge: 0,
        }],
        prest_files: (0..1200).map(|d| (d, 1)).collect(),
        specials: Specials::shipped(),
    }
}

/// Monster class 0: AI 1 (Idle), aidel 15, spawnable in packs of one,
/// rarity 1, no minions.
pub fn monster_class() -> Monstats {
    let mut m = monster_rec();
    m.ai = 1;
    m.aidel = 15;
    m.aidel_n = 15;
    m.aidel_h = 15;
    m.skill1 = 0xFFFF;
    m.skill2 = 0xFFFF;
    m.skill3 = 0xFFFF;
    m.rarity = 1;
    m.mingrp = 1;
    m.maxgrp = 1;
    m.minion1 = 0xFFFF;
    m.minion2 = 0xFFFF;
    m.enabled = true;
    m.isspawn = true;
    m
}

/// levels.txt rows: no monsters, except [`ISLE`]: class 0, MonDen
/// 10000.
pub fn levels() -> Vec<Levels> {
    let mut v: Vec<Levels> = vec![blank(); 150];
    for (i, l) in v.iter_mut().enumerate() {
        for m in [
            &mut l.mon1,
            &mut l.mon2,
            &mut l.mon3,
            &mut l.mon4,
            &mut l.mon5,
            &mut l.mon6,
            &mut l.mon7,
            &mut l.mon8,
            &mut l.mon9,
            &mut l.mon10,
            &mut l.mon11,
            &mut l.mon12,
            &mut l.mon13,
            &mut l.mon14,
            &mut l.mon15,
            &mut l.mon16,
            &mut l.mon17,
            &mut l.mon18,
            &mut l.mon19,
            &mut l.mon20,
            &mut l.mon21,
            &mut l.mon22,
            &mut l.mon23,
            &mut l.mon24,
            &mut l.mon25,
            &mut l.nmon1,
            &mut l.umon1,
        ] {
            *m = 0xFFFF;
        }
        l.act = crate::drlg::act_of_level(i as u32);
    }
    let g = &mut v[ISLE as usize];
    g.mon1 = 0;
    g.nummon = 1;
    g.monden = 10_000;
    g.monden_n = 10_000;
    g.monden_h = 10_000;
    v
}

pub fn world_tables() -> WorldTables {
    let monstats = vec![monster_class()];
    let monstats2: Vec<Monstats2> = vec![blank()];
    let levels = levels();
    WorldTables {
        pop: PopTables::from_records(&levels, &monstats, &monstats2, &[]),
        monstats,
        monstats2,
        monlvl: vec![blank::<Monlvl>(); 10],
        levels,
        difficultylevels: vec![blank::<Difficultylevels>(); 3],
        monstats_extra: vec![MonstatsExtra::default()],
        components: vec![[0; 16]],
        ..WorldTables::default()
    }
}

fn action_tables() -> ActionTables {
    ActionTables {
        missiles: Vec::new(),
        skills: skill_tables(vec![skill_rec()]),
        combat: combat_tables(vec![monster_class()]),
        levels: levels(),
        skill_modes: vec![[0; 8]],
        overlay_count: 0,
        monequip: Vec::new(),
        arena: Vec::new(),
    }
}

fn unit_data() -> UnitData {
    UnitData {
        monsters: vec![MonsterInfo {
            enabled: true,
            aidel: [15, 15, 15],
            moves: 1 << 4,
            mode_chart: false,
        }],
        ..UnitData::default()
    }
}

// ---- the fixture --------------------------------------------------------------------------

/// A game whose act 0 DRLG was created through [`WorldTypes`] (the
/// outdoor placer, preset inits), and the combined systems.
pub struct Fx {
    pub game: Game,
    pub sim: WorldSim<TestPending>,
}

impl Fx {
    /// `ds1s`: DS1 files by path (`def<N>.ds1`).
    pub fn new(ds1s: Ds1s) -> Self {
        Self::with_tables(ds1s, |_| {})
    }

    /// As [`Fx::new`], with population / init tables changed by `tweak`.
    pub fn with_tables(ds1s: Ds1s, tweak: impl FnOnce(&mut WorldTables)) -> Self {
        Self::with_data(ds1s, |_, _| {}, tweak)
    }

    /// As [`Fx::with_tables`], with the DRLG and lvlprest views changed
    /// by `drlg` first.
    pub fn with_data(
        ds1s: Ds1s,
        drlg: impl FnOnce(&mut DrlgData, &mut PresetData),
        tweak: impl FnOnce(&mut WorldTables),
    ) -> Self {
        let mut dd = drlg_data();
        let mut pd = preset_data();
        drlg(&mut dd, &mut pd);
        let data = Arc::new(dd);
        let types = SharedTypes::new(WorldTypes::new(
            data.clone(),
            Maze::new(maze_data()),
            pd.clone(),
            outdoor_data(&pd),
            Box::new(ds1s),
            Box::new(SubFileMap::default()),
        ));
        let mut handle = types.clone();
        let drlg = Drlg::create(0, INIT, 0, 0, false, &data, &mut handle).expect("act 0");
        let mut dungeon = crate::drlg::Dungeon::default();
        dungeon.acts[0] = Some(drlg);
        let world = DrlgWorld {
            dungeon,
            data,
            tiles: Box::new(tiles()),
            types: Box::new(handle),
        };
        let hooks = ActionHooks::new(
            Arc::new(action_tables()),
            world,
            Seed::init_low(GAME_SEED),
            TestPending::default(),
        );
        let mut tables = world_tables();
        tweak(&mut tables);
        let state = WorldState::new(
            types,
            Arc::new(tables),
            crate::monsters::init::GameInfo::default(),
        );
        let sim = WorldSim::new(crate::stats::tests::data(), unit_data(), hooks, state);
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        Self { game, sim }
    }

    /// The act 0 DRLG.
    pub fn drlg(&self) -> &Drlg {
        self.sim.action.sys.hooks.drlg.dungeon.acts[0]
            .as_ref()
            .expect("act 0")
    }

    pub fn drlg_mut(&mut self) -> &mut Drlg {
        self.sim.action.sys.hooks.drlg.dungeon.acts[0]
            .as_mut()
            .expect("act 0")
    }

    /// Runs `f` on act 0's DRLG and its services.
    pub fn with_act<R>(
        &mut self,
        f: impl FnOnce(&mut Drlg, &mut Services<'_>) -> Result<R, DrlgError>,
    ) -> Result<R, DrlgError> {
        self.sim
            .action
            .sys
            .hooks
            .drlg
            .with_act(0, &mut self.game.lists, f)
            .expect("act 0")
    }

    /// Get-or-allocate `id` and generate its rooms.
    pub fn generate(&mut self, id: u32) -> Result<(LevelIdx, Vec<DrlgRoomId>), DrlgError> {
        self.with_act(|d, svc| {
            let l = d.get_or_alloc_level(svc.data, svc.types, id)?;
            d.generate_level(svc.data, svc.types, l)?;
            Ok((l, d.level_rooms(l)))
        })
    }

    /// Streams DRLG rooms in order; their active rooms.
    pub fn stream(&mut self, rooms: &[DrlgRoomId]) -> Result<Vec<RoomId>, DrlgError> {
        self.with_act(|d, svc| {
            let mut out = Vec::new();
            for &r in rooms {
                out.push(d.stream_room(svc, r)?.expect("active"));
            }
            Ok(out)
        })
    }

    /// The level types' state.
    pub fn types(&self) -> std::cell::Ref<'_, WorldTypes> {
        self.sim.world.types.borrow()
    }

    /// No adapter, level-type or unit-dispatch error so far.
    pub fn assert_clean(&self) {
        assert_eq!(self.sim.errors(), Vec::<String>::new());
    }
}

/// [`ISLE`]'s DS1 (40 × 18) with one monster of class 0.
pub fn isle_ds1s() -> Ds1s {
    let mut m = BTreeMap::new();
    let (x, y) = ISLE_MONSTER;
    m.insert(
        format!("def{ISLE_DEF}.ds1").into_bytes(),
        ds1(40, 18, &[(0, x, y)]),
    );
    Ds1s(m)
}

/// `seed` advanced by `n` steps.
pub fn stepped(mut seed: Seed, n: usize) -> Seed {
    for _ in 0..n {
        seed.step();
    }
    seed
}
mod creation;
