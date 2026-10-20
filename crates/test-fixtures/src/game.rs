//! A game from a loaded table set: [`GameData`] holds the loaded and
//! fixed-up tables, the level-type views and every DRLG file
//! (`d2_server::world_data`), and builds what the server hands the sim:
//! [`ActionTables`], [`WorldTables`], the vitals, stat and unit data,
//! the shared level types, an act's [`DrlgWorld`] and a [`WorldSim`].
//!
//! The builders are the ones the game-file tests use on the live set
//! (`d2-server/tests/game_wired_host.rs`), gathered here so a synthetic
//! install ([`crate::install`]) goes the same way; nothing here adds a
//! rule. The seams with no provider keep the sim's defaults: [`Seams`]
//! answers only the bookkeeping no rule decides (the transport), as the
//! live host test does.

use std::sync::Arc;

use d2_data::bin::{BinSet, BinTable};
use d2_data::fixup::{self, FixedSet};
use d2_data::tables::{
    decode_all, Difficultylevels, Levels, Missiles, Monequip, Monlvl, Monprop, Monstats, Monstats2,
    Monumod, Objects, Record, Superuniques,
};
use d2_formats::animdata::AnimData;
use d2_formats::mpq::ArchiveSet;
use d2_server::adapters::handlers::world::{ActionWorld, Outbox};
use d2_server::adapters::SimGame;
use d2_server::world_data::tables::LevelTables;
use d2_server::world_data::{archive, WorldDataError, WorldFiles};
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::combat::CombatTables;
use d2_sim::drlg::maze::Maze;
use d2_sim::drlg::{Drlg, DrlgData, DrlgError, Dungeon, NoLevelTypes};
use d2_sim::monsters::ai::skill_modes;
use d2_sim::monsters::init::{component_counts, monstats_extra, GameInfo, NamedIds};
use d2_sim::monsters::population::PopTables;
use d2_sim::rng::Seed;
use d2_sim::skills::{SkillTables, LEVEL_CAP_114D};
use d2_sim::stats::{StatData, StateTable};
use d2_sim::units::hooks::UnitData;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionTables, DrlgWorld, Pending};
use d2_sim::wiring::worldgen::dispatch::WorldSim;
use d2_sim::wiring::worldgen::levels::{SharedTypes, WorldTypes};
use d2_sim::wiring::worldgen::{WorldPending, WorldState, WorldTables};
use d2_sim::world::waypoints::WaypointData;

use crate::install::Install;

/// A step of the game build failed.
#[derive(Debug, thiserror::Error)]
pub enum GameError {
    /// A table view could not be built from the set.
    #[error("{what}: {detail}")]
    Table { what: String, detail: String },
    #[error(transparent)]
    WorldData(#[from] WorldDataError),
    #[error("act {act}: {source:?}")]
    Drlg { act: u8, source: DrlgError },
}

/// How act 0's DRLG is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActCreation {
    /// `Drlg::create` over the level types: the act placer, then the
    /// town. What a game on the live tables does.
    Full,
    /// `Drlg::create` without the act placer (the placer runs over
    /// `NoLevelTypes`, no town), then the town allocated and generated
    /// over the level types. For the synthetic set: the Act I placer
    /// (`place.rs` A1W / A1M) links levels 1–7, 17, 26 and 39 by the
    /// game's level ids and types, which an 8-row made-up levels table
    /// does not have. The DRLG seed then skips the placer's draws: a
    /// fixture state, not one a real game reaches.
    TownOnly,
}

fn err(what: &str, detail: impl std::fmt::Display) -> GameError {
    GameError::Table {
        what: what.to_owned(),
        detail: detail.to_string(),
    }
}

/// Everything a game is built from.
#[derive(Clone, Debug)]
pub struct GameData {
    /// The loaded set (`bin::load`): the skill, combat and vitals views
    /// read it.
    pub bins: BinSet,
    pub anim: AnimData,
    /// The fixed-up set (`fixup::apply`): every other view.
    pub fixed: FixedSet,
    pub level: LevelTables,
    pub files: WorldFiles,
}

impl GameData {
    /// From a loaded set and the archives it came from: `AnimData.d2`,
    /// the fix-ups, the level-type views and every DRLG file the views
    /// name (`WorldFiles::load`; a missing or bad file is an error).
    pub fn load(bins: BinSet, archives: &ArchiveSet) -> Result<Self, GameError> {
        let anim = fixup::read_animdata(archives).map_err(|e| err("AnimData.d2", e))?;
        let fixed = fixup::apply(&bins, &anim).map_err(|e| err("fixup", e))?;
        let level = LevelTables::from_fixed(&fixed)?;
        let files = WorldFiles::load(
            &level.drlg,
            &level.preset,
            &level.outdoor,
            archive::reader(archives),
        )?;
        Ok(Self {
            bins,
            anim,
            fixed,
            level,
            files,
        })
    }

    /// From a built synthetic install.
    pub fn from_install(i: &Install) -> Result<Self, GameError> {
        Self::load(i.loaded.clone(), &i.archives)
    }

    /// A fixed-up table by name.
    pub fn table(&self, name: &str) -> Result<&BinTable, GameError> {
        self.fixed
            .table(name)
            .ok_or_else(|| err(name, "not in the set"))
    }

    /// A fixed-up table's typed rows.
    pub fn rows<T: Record>(&self) -> Result<Vec<T>, GameError> {
        decode_all(self.table(T::TABLE)?).map_err(|e| err(T::TABLE, e))
    }

    /// Missiles, skills, combat, levels and the monster skill modes.
    pub fn action_tables(&self) -> Result<ActionTables, GameError> {
        Ok(ActionTables {
            missiles: self.rows::<Missiles>()?,
            skills: SkillTables::from_bin(&self.bins, LEVEL_CAP_114D)
                .map_err(|e| err("skills", e))?,
            combat: CombatTables::from_bin(&self.bins).map_err(|e| err("combat", e))?,
            levels: self.rows::<Levels>()?,
            skill_modes: skill_modes(self.table("monstats")?),
            overlay_count: i32::try_from(self.table("overlay")?.count).unwrap_or(i32::MAX),
            monequip: self.rows::<Monequip>()?,
            arena: self.rows::<d2_data::tables::Arena>()?,
        })
    }

    /// The monsters' skill sequences (from `monseq`); a set without `monseq` (a synthetic install) has no lists.
    pub fn monster_sequences(
        &self,
    ) -> Result<d2_sim::skills::sequences::MonsterSequences, GameError> {
        let monseq = match self.fixed.table("monseq") {
            Some(_) => self.rows::<d2_data::tables::Monseq>()?,
            None => Vec::new(),
        };
        Ok(d2_sim::skills::sequences::MonsterSequences::from_tables(
            self.table("monstats")?,
            &monseq,
        ))
    }

    /// The object code's tables (`objects`, `shrines`, `levels`).
    pub fn object_tables(&self) -> Result<d2_sim::world::objects::ObjectTables, GameError> {
        Ok(d2_sim::world::objects::ObjectTables {
            objects: self.rows::<d2_data::tables::Objects>()?,
            shrines: self.rows::<d2_data::tables::Shrines>()?,
            levels: self.rows::<Levels>()?,
            objgroup: self.rows::<d2_data::tables::Objgroup>()?,
            leveldefs: self.rows::<d2_data::tables::Leveldefs>()?,
        })
    }

    /// Population and monster-init tables.
    pub fn world_tables(&self) -> Result<WorldTables, GameError> {
        let levels = self.rows::<Levels>()?;
        let monstats = self.rows::<Monstats>()?;
        let monstats2 = self.rows::<Monstats2>()?;
        let superuniques = self.rows::<Superuniques>()?;
        Ok(WorldTables {
            pop: PopTables::from_records(&levels, &monstats, &monstats2, &superuniques)
                .with_bins(self.table("monstats")?, self.table("monstats2")?),
            monstats,
            monstats2,
            monlvl: self.rows::<Monlvl>()?,
            levels,
            monprop: self.rows::<Monprop>()?,
            monequip: self.rows::<Monequip>()?,
            monumod: self.rows::<Monumod>()?,
            superuniques,
            difficultylevels: self.rows::<Difficultylevels>()?,
            monstats_extra: monstats_extra(self.table("monstats")?),
            components: component_counts(self.table("monstats2")?),
            ids: NamedIds::default(),
            montype_equiv: self.fixed.montype_equiv.clone(),
        })
    }

    pub fn vitals(&self) -> Result<VitalsTables, GameError> {
        VitalsTables::from_bin(&self.bins).map_err(|e| err("vitals", e))
    }

    pub fn stat_data(&self) -> Result<StatData, GameError> {
        let states = StateTable::new(self.table("states")?, &self.fixed.states)
            .map_err(|e| err("states", e))?;
        StatData::new(
            self.table("itemstatcost")?,
            self.table("charstats")?,
            states,
            self.table("monstats")?,
            self.table("skills")?,
        )
        .map_err(|e| err("stat data", e))
    }

    /// Unit data of an expansion game.
    pub fn unit_data(&self) -> Result<UnitData, GameError> {
        let u = UnitData::new(self.table("monstats")?, self.table("monstats2")?)
            .map_err(|e| err("unit data", e))?;
        Ok(UnitData {
            expansion: true,
            ..u
        })
    }

    /// The waypoint tables of `ActionWorld`.
    pub fn waypoints(&self) -> Result<WaypointData, GameError> {
        Ok(WaypointData::new(
            &self.rows::<Levels>()?,
            &self.rows::<Objects>()?,
        ))
    }

    /// The DRLG data and the level-type dispatcher over the providers.
    pub fn level_types(&self) -> (Arc<DrlgData>, SharedTypes) {
        let data = Arc::new(self.level.drlg.clone());
        let types = SharedTypes::new(WorldTypes::new(
            data.clone(),
            Maze::new(self.level.maze.clone()),
            self.level.preset.clone(),
            self.level.outdoor.clone(),
            Box::new(self.files.ds1.clone()),
            Box::new(self.files.subs.clone()),
        ));
        (data, types)
    }

    /// Act 0 created on `init_seed` with `town` as its town level
    /// ([`ActCreation`]), over the level types; the DT1 provider.
    pub fn drlg_world(
        &self,
        data: Arc<DrlgData>,
        types: &SharedTypes,
        creation: ActCreation,
        init_seed: u32,
        town: u32,
    ) -> Result<DrlgWorld, GameError> {
        self.drlg_world_in(data, types, creation, init_seed, 0, town)
    }

    /// [`Self::drlg_world`] of act `act` (0..=4; `town` is that act's
    /// town level).
    pub fn drlg_world_act(
        &self,
        data: Arc<DrlgData>,
        types: &SharedTypes,
        creation: ActCreation,
        init_seed: u32,
        act: u8,
        town: u32,
    ) -> Result<DrlgWorld, GameError> {
        self.drlg_world_full(data, types, creation, init_seed, 0, act, town)
    }

    /// [`Self::drlg_world`] on a game `difficulty` (0 normal, 1
    /// nightmare, 2 hell; `Drlg::create`'s difficulty).
    pub fn drlg_world_in(
        &self,
        data: Arc<DrlgData>,
        types: &SharedTypes,
        creation: ActCreation,
        init_seed: u32,
        difficulty: u8,
        town: u32,
    ) -> Result<DrlgWorld, GameError> {
        self.drlg_world_full(data, types, creation, init_seed, difficulty, 0, town)
    }

    #[allow(clippy::too_many_arguments)]
    fn drlg_world_full(
        &self,
        data: Arc<DrlgData>,
        types: &SharedTypes,
        creation: ActCreation,
        init_seed: u32,
        difficulty: u8,
        act: u8,
        town: u32,
    ) -> Result<DrlgWorld, GameError> {
        let drlg_err = |source| GameError::Drlg { act, source };
        let mut handle = types.clone();
        let drlg = match creation {
            ActCreation::Full => {
                Drlg::create(act, init_seed, difficulty, town, false, &data, &mut handle)
                    .map_err(drlg_err)?
            }
            ActCreation::TownOnly => {
                let mut d = Drlg::create(
                    act,
                    init_seed,
                    difficulty,
                    0,
                    false,
                    &data,
                    &mut NoLevelTypes,
                )
                .map_err(drlg_err)?;
                let l = d
                    .get_or_alloc_level(&data, &mut handle, town)
                    .map_err(drlg_err)?;
                d.generate_level(&data, &mut handle, l).map_err(drlg_err)?;
                d
            }
        };
        let mut dungeon = Dungeon::default();
        dungeon.acts[usize::from(act)] = Some(drlg);
        Ok(DrlgWorld {
            dungeon,
            data,
            tiles: Box::new(self.files.dt1.clone()),
            types: Box::new(handle),
        })
    }

    /// A `WorldSim` of an expansion game: act 0 ([`Self::drlg_world`]),
    /// the action hooks on `game_seed` with AnimData, vitals and the path
    /// provider, the world state, the population regions and the object
    /// control created, in the game's seed order (the NPC and quest
    /// controls, the next two creation seeds, are the caller's:
    /// `WorldSim::create_game` runs all four).
    /// Returns the sim and the shared level types (preset lookups).
    pub fn world_sim<X: WorldPending>(
        &self,
        creation: ActCreation,
        init_seed: u32,
        town: u32,
        game_seed: u32,
        x: X,
    ) -> Result<(WorldSim<X>, SharedTypes), GameError> {
        self.world_sim_act(creation, init_seed, 0, town, game_seed, x)
    }

    /// [`Self::world_sim`] whose created act is `act`.
    pub fn world_sim_act<X: WorldPending>(
        &self,
        creation: ActCreation,
        init_seed: u32,
        act: u8,
        town: u32,
        game_seed: u32,
        x: X,
    ) -> Result<(WorldSim<X>, SharedTypes), GameError> {
        let (data, types) = self.level_types();
        let world = self.drlg_world_act(data, &types, creation, init_seed, act, town)?;
        let mut hooks = ActionHooks::new(
            Arc::new(self.action_tables()?),
            world,
            Seed::init_low(game_seed),
            x,
        );
        hooks.anim_data = Some(Arc::new(self.anim.clone()));
        hooks.monster_sequences = Some(Arc::new(self.monster_sequences()?));
        hooks.vitals = Some(Arc::new(self.vitals()?));
        hooks.enable_paths().map_err(|e| err("paths", e))?;
        let info = GameInfo {
            expansion: true,
            ..GameInfo::default()
        };
        let state = WorldState::new(types.clone(), Arc::new(self.world_tables()?), info);
        let mut sim = WorldSim::new(Arc::new(self.stat_data()?), self.unit_data()?, hooks, state);
        sim.create_regions();
        sim.create_objects(Arc::new(self.object_tables()?));
        Ok((sim, types))
    }
}

/// The seams without a provider keep their defaults (`Pending`,
/// `WorldPending`), except the transport (`send` collected and handed
/// to the host by [`Outbox`]).
#[derive(Default)]
pub struct Seams {
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// The animation names (unit type, class, mode → AnimData record
    /// name). `None`: no unit has an animation, so modes don't animate.
    /// When set, a named record plays at its AnimData speed.
    pub anim_names: Option<AnimNames>,
    /// Staged aim for a fixture skill use.
    pub skill_target: Option<(i32, i32)>,
}

/// Names a unit's animation record ([`Seams::anim_names`]).
pub type AnimNames = fn(UnitType, u32, u32) -> Option<[u8; 8]>;

impl Pending for Seams {
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn anim_name(&self, _: UnitId, ty: UnitType, class: u32, mode: u32) -> Option<[u8; 8]> {
        self.anim_names.and_then(|f| f(ty, class, mode))
    }
    fn anim_rate(&self, _: UnitId, speed: Option<u32>) -> i16 {
        match self.anim_names {
            Some(_) => speed.map_or(0, |s| s as i16),
            None => 0,
        }
    }
}

impl WorldPending for Seams {}

impl Outbox for Seams {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// The server game over a [`WorldSim`] with [`Seams`].
pub type Sim = SimGame<WorldSim<Seams>, ActionWorld>;

mod skill_use;
