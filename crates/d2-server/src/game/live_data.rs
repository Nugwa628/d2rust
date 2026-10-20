//! The game data from the user's files: [`GameData`], [`LiveData`], the
//! waypoint tables and the level sources of the DRLG.

use super::*;

/// The tables the game's waypoint world reads, and the waypoint object's
/// class.
#[derive(Debug, Clone)]
pub struct WaypointTables {
    pub levels: Vec<Levels>,
    pub objects: Vec<Objects>,
    pub object_class: u32,
}

impl WaypointTables {
    /// The user's own `levels` and `objects` tables (the live `.bin` set,
    /// validated).
    pub fn live(archives: &dyn TableFiles) -> Result<Self, BuildError> {
        let set = d2_data::bin::load_from(archives, "eng")
            .map_err(|e| BuildError::Tables(e.to_string()))?;
        let table = |name: &str| {
            set.table(name)
                .ok_or_else(|| BuildError::Tables(format!("{name} not loaded")))
        };
        let levels: Vec<Levels> =
            decode_all(table("levels")?).map_err(|e| BuildError::Tables(e.to_string()))?;
        let objects: Vec<Objects> =
            decode_all(table("objects")?).map_err(|e| BuildError::Tables(e.to_string()))?;
        let object_class = objects
            .iter()
            .position(|o| o.operatefn == 23 && o.initfn == 17)
            .ok_or(BuildError::NoWaypointObject)? as u32;
        Ok(WaypointTables {
            levels,
            objects,
            object_class,
        })
    }
}

/// Every DS1 / DT1 the level types read, through the typed reads of
/// `files` (archives or native alike).
fn load_world_files(files: &GameFiles, levels: &LevelTables) -> Result<WorldFiles, BuildError> {
    let read = |path: &[u8]| -> Result<NativeAsset, world_data::WorldDataError> {
        let name = world_data::file_name(path)?;
        let canon = CanonicalPath::new(&name).map_err(|e| world_data::WorldDataError::Read {
            path: name.clone(),
            detail: e.to_string(),
        })?;
        match files.read_native(&canon) {
            Some(Ok(a)) => Ok(a),
            Some(Err(detail)) => Err(world_data::WorldDataError::Read { path: name, detail }),
            None => Err(world_data::WorldDataError::Read {
                path: name,
                detail: "in no archive".into(),
            }),
        }
    };
    Ok(WorldFiles::load_typed(
        &levels.drlg,
        &levels.preset,
        &levels.outdoor,
        |p| match read(p)? {
            NativeAsset::Ds1(d) => Ok(d),
            _ => Err(world_data::WorldDataError::BadPath(p.to_vec())),
        },
        |p| match read(p)? {
            NativeAsset::Dt1(d) => Ok(d),
            _ => Err(world_data::WorldDataError::BadPath(p.to_vec())),
        },
    )?)
}

/// `ExpField.D2`: the walk-back field the floor
/// drop of items, monsters' and chests' alike, tests its spots with.
fn load_expfield(files: &GameFiles) -> Result<Arc<d2_sim::path::search::ExpField>, BuildError> {
    use d2_sim::path::search::ExpField;
    let err = |m: String| BuildError::Tables(format!("{}: {m}", ExpField::PATH));
    let path = CanonicalPath::new(ExpField::PATH).map_err(|e| err(e.to_string()))?;
    match files.read_native(&path) {
        Some(Ok(NativeAsset::ExpField(f))) => ExpField::from_cells(f.height, f.width, f.cells)
            .map(Arc::new)
            .ok_or_else(|| err("cell count does not match its header".into())),
        Some(Ok(_)) => Err(err("wrong kind".into())),
        Some(Err(e)) => Err(err(e.to_string())),
        None => Err(err("in no archive".into())),
    }
}

/// Everything the game reads from the user's files, loaded up front.
#[derive(Debug)]
pub struct LiveData {
    pub waypoints: WaypointTables,
    /// drlg-data's level-type table views (`LevelTables::from_fixed`).
    pub levels: LevelTables,
    /// Every DS1 / DT1 the level types read, parsed.
    pub files: WorldFiles,
    /// The loaded and fixed-up sets and `AnimData.d2`: every other table
    /// view of the game.
    pub tables: GameTables,
    /// The chest drop's tables (`ActionHooks::object_drops`).
    pub drops: Arc<DropTables>,
    /// The item class picks of the drop helpers (`DeathDrops::with_picks`).
    pub picks: Arc<d2_sim::treasure::class_pick::ClassPicks>,
    /// `InteractionState::hireling_tables`.
    pub hirelings: HirelingTables,
    /// The `.d2s` reader's tables (`--save`), for the app's expansion game.
    pub save: SaveData,
    /// The floor drop's walk-back field (`data\global\ExpField.D2`), set on the
    /// path provider of every game.
    pub expfield: Arc<d2_sim::path::search::ExpField>,
    /// The archive set itself (the client's other readers: sounds).
    pub archives: Arc<GameFiles>,
}

impl LiveData {
    /// Loads the table sets, the waypoint, drop, hireling and save tables
    /// and the level data (a table or a named file that is missing or does
    /// not parse is an error; nothing falls back to synthetic data).
    pub fn load(archives: Arc<GameFiles>) -> Result<Self, BuildError> {
        let waypoints = WaypointTables::live(archives.as_ref())?;
        // The layers' table patches are already applied by the file
        // source. A mod game's table extensions (the extension columns of
        // `addcol` and the calc keyword registry, `docs/plans/mods/seams.md`
        // MS4 and MS9) are to be carried into the loaded tables here, passed
        // in with the game's source ([`GameSource`]), once those seams exist.
        let bins = d2_data::bin::load_from(archives.as_ref(), d2_data::bin::DEFAULT_LANGUAGE)
            .map_err(|e| BuildError::Tables(e.to_string()))?;
        let anim = match archives.read_native(
            &CanonicalPath::new(d2_formats::animdata::PATH)
                .map_err(|e| BuildError::Tables(format!("{}: {e}", d2_formats::animdata::PATH)))?,
        ) {
            Some(Ok(NativeAsset::AnimData(a))) => a,
            Some(Ok(_)) => return Err(BuildError::Tables("AnimData.d2: wrong kind".into())),
            Some(Err(e)) => return Err(BuildError::Tables(format!("AnimData.d2: {e}"))),
            None => return Err(BuildError::Tables("AnimData.d2: in no archive".into())),
        };
        let expfield = load_expfield(&archives)?;
        let tables = GameTables::from_loaded(bins, anim)?;
        let levels = LevelTables::from_fixed(&tables.fixed)?;
        let files = load_world_files(&archives, &levels)?;
        Ok(LiveData {
            waypoints,
            levels,
            files,
            drops: Arc::new(drop_tables(&tables.fixed)?),
            picks: Arc::new(class_picks(&tables.fixed)?),
            hirelings: hireling_tables(&tables.fixed)?,
            save: SaveData::from_fixed(&tables.fixed, GAME_SETUP.expansion)?,
            tables,
            expfield,
            archives,
        })
    }

    /// Reads and checks the save `bytes` on the app's game
    /// (`d2s::read`, with the header checks against [`GAME_SETUP`]:
    /// Normal, expansion, the character's own hardcore mode). The
    /// client's name is the save's own: the client sends the selected
    /// character's name when it joins ([`create_request_for`]).
    ///
    /// A full save that still carries the creation stub's "new character"
    /// flag (internal code 2, written by game versions whose first save
    /// kept the flag) is repaired and read again
    /// ([`d2s::clear_stale_new_flag`]): the character loads, and its next
    /// save is written without the flag.
    pub fn read_save(&self, bytes: &[u8], difficulty: u8) -> Result<D2s, d2s::D2sError> {
        match self.read_save_strict(bytes, difficulty) {
            Err(e) if e.internal() == Some(2) => match d2s::clear_stale_new_flag(bytes) {
                Some(fixed) => self.read_save_strict(&fixed, difficulty),
                None => Err(e),
            },
            r => r,
        }
    }

    fn read_save_strict(&self, bytes: &[u8], difficulty: u8) -> Result<D2s, d2s::D2sError> {
        let opts = ReadOptions {
            expansion: GAME_SETUP.expansion,
            game: Some(d2s::GameContext {
                client_name: save_name(bytes).to_vec(),
                expansion: GAME_SETUP.expansion,
                // The character's own mode: a hardcore character plays a
                // hardcore game (a dead one is refused by the header
                // check: permanent death).
                hardcore: d2s::header_status(bytes).is_some_and(|s| s & d2s::status::HARDCORE != 0),
                difficulty,
            }),
        };
        d2s::read(bytes, &opts, &self.save)
    }
}

/// Where the game's tables and levels come from.
#[derive(Debug, Clone)]
pub enum GameData {
    /// Tables and level data already loaded from the user's files.
    Live(Arc<LiveData>),
}

impl GameData {
    /// The data of `game_dir` (`$D2_GAME_DIR`): the user's own files. No
    /// directory is an error naming `D2_GAME_DIR` (nothing plays on
    /// invented data, M23), and so is a directory that does not load.
    pub fn select(game_dir: Option<&std::path::Path>) -> Result<Self, BuildError> {
        let dir = game_dir.ok_or(BuildError::NoGameDir)?;
        let archives = ArchiveSet::open_dir(dir).map_err(|e| BuildError::Archives {
            dir: dir.display().to_string(),
            message: e.to_string(),
        })?;
        Ok(GameData::Live(Arc::new(LiveData::load(Arc::new(
            GameFiles::archives(Arc::new(archives)),
        ))?)))
    }

    /// The data of a converted native folder (`play --native DIR`): the same
    /// tables and level files, read from the native source.
    pub fn select_native(dir: &std::path::Path) -> Result<Self, BuildError> {
        Self::select_native_with(dir, &Layers::OrderFile)
    }

    /// [`GameData::select_native`] with the layers the game chooses
    /// ([`Layers`]: a game's builder passes its own list).
    pub fn select_native_with(dir: &std::path::Path, layers: &Layers) -> Result<Self, BuildError> {
        let files = GameFiles::native_with(dir, layers.order()).map_err(|message| {
            BuildError::Archives {
                dir: dir.display().to_string(),
                message,
            }
        })?;
        Ok(GameData::Live(Arc::new(LiveData::load(Arc::new(files))?)))
    }

    pub fn tables(&self) -> WaypointTables {
        let GameData::Live(d) = self;
        d.waypoints.clone()
    }
}

/// The level generation the game runs on: table view, tile library, level
/// types, and per act the DRLG init seed and the server's town level id.
pub struct LevelSource {
    pub data: Arc<DrlgData>,
    pub tiles: Box<dyn TileSource>,
    pub types: Box<dyn LevelTypes>,
    /// The same level-type state as `types`, for the world state's
    /// preset lookups (population reads the rooms the DRLG generated).
    pub shared: SharedTypes,
    /// (act, init seed, town level id) of each created act.
    pub acts: Vec<(u8, u32, u32)>,
}

impl LevelSource {
    /// Build fresh level-type state from the loaded files, using the same
    /// initialization seed for every act and each act's town level.
    pub fn live(d: &LiveData, init_seed: u32) -> Self {
        let data = Arc::new(d.levels.drlg.clone());
        let types = SharedTypes::new(WorldTypes::new(
            data.clone(),
            Maze::new(d.levels.maze.clone()),
            d.levels.preset.clone(),
            d.levels.outdoor.clone(),
            Box::new(d.files.ds1.clone()),
            Box::new(d.files.subs.clone()),
        ));
        LevelSource {
            data,
            tiles: Box::new(d.files.dt1.clone()),
            shared: types.clone(),
            types: Box::new(types),
            acts: vec![
                (0, init_seed, 1),
                (1, init_seed, ACT2_TOWN),
                (2, init_seed, KURAST_DOCKS),
                (4, init_seed, ACT5_TOWN),
                (3, init_seed, PANDEMONIUM_FORTRESS),
            ],
        }
    }
}

pub struct LiveDs1(pub Arc<LiveData>);

impl Ds1Source for LiveDs1 {
    fn ds1(&self, path: &[u8]) -> Option<&Ds1Input> {
        self.0.files.ds1.ds1(path)
    }
}

/// The live lvlsub files of the client DRLG's level types.
pub struct LiveSubs(pub Arc<LiveData>);

impl SubFiles for LiveSubs {
    fn sub_file(&self, file: &[u8]) -> Option<&SubFile> {
        self.0.files.subs.sub_file(file)
    }
}
