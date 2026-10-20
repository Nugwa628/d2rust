//! Source selection and character codecs for the headless game.
use super::*;
use d2_formats::d2s::SaveTables;
use std::fmt::Debug;
use std::path::PathBuf;
/// The layers a converted native folder stacks over its `base/`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Layers {
    /// `base/`, then the mods `mods/order.toml` lists, in its order.
    #[default]
    OrderFile,
    /// `base/` and exactly these folders under `mods/`, in this order;
    /// `mods/order.toml` is not read. Empty is `base/` alone.
    List(Vec<String>),
}

impl Layers {
    /// `base/` alone.
    pub fn base() -> Layers {
        Layers::List(Vec::new())
    }

    /// The explicit mod list, or `None` for the order file.
    pub fn order(&self) -> Option<&[String]> {
        match self {
            Layers::OrderFile => None,
            Layers::List(mods) => Some(mods),
        }
    }
}

/// Where a game reads the player's files from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameSource {
    /// The game's install folder and its archives (`--game-dir`).
    Archives(PathBuf),
    /// A converted native folder (`--native`) with the layers the game
    /// stacks on it.
    Native(PathBuf, Layers),
}

impl GameData {
    /// Loads the game's tables and level data from `source`.
    pub fn open(source: &GameSource) -> Result<GameData, BuildError> {
        match source {
            GameSource::Archives(dir) => GameData::select(Some(dir)),
            GameSource::Native(dir, layers) => GameData::select_native_with(dir, layers),
        }
    }
}

/// A game's character save format: the bytes of its save files, read
/// into and written from the character the game plays ([`D2s`]). A mod
/// game whose characters hold more than D2RUST's keeps its own data in
/// its own format through its own codec.
pub trait CharacterCodec: Send + Sync + Debug {
    /// The save files' extension, without the dot (`d2s`).
    fn extension(&self) -> &str;

    /// Reads the save `bytes` for a game on `difficulty`, with the game's
    /// tables. The error is a message for the player.
    fn read(&self, data: &LiveData, bytes: &[u8], difficulty: u8) -> Result<D2s, String>;

    /// The bytes of `save`. The error is a message for the player.
    fn write(&self, save: &D2s, tables: &dyn SaveTables) -> Result<Vec<u8>, String>;
}

/// D2RUST's own character format: the `.d2s` reader with the app game's
/// header checks ([`LiveData::read_save`]) and the `.d2s` writer.
#[derive(Debug, Clone, Copy, Default)]
pub struct D2sCodec;

impl CharacterCodec for D2sCodec {
    fn extension(&self) -> &str {
        "d2s"
    }

    fn read(&self, data: &LiveData, bytes: &[u8], difficulty: u8) -> Result<D2s, String> {
        data.read_save(bytes, difficulty).map_err(|e| e.to_string())
    }

    fn write(&self, save: &D2s, tables: &dyn SaveTables) -> Result<Vec<u8>, String> {
        d2s::write(save, tables).map_err(|e| e.to_string())
    }
}

/// A wired game built on its owning thread, with file-backed animation names.
/// File loading, rules and character selection are independent of any client.
pub struct GameBuilder {
    pub data: GameData,
    pub seed: u32,
    pub character: Character,
    pub rules: Ruleset,
    appearance: Option<Option<Arc<dyn crate::local_seams::UnitAppearance>>>,
}
impl GameBuilder {
    /// Start from the normal game defaults and file-backed animation names.
    pub fn new(data: GameData) -> Self {
        Self {
            data,
            seed: DEFAULT_SEED,
            character: Character::New,
            rules: Ruleset::Original,
            appearance: None,
        }
    }
    /// Override the file-backed animation names, including explicitly disabling them.
    /// The client supplies its existing appearance table through this seam.
    pub fn appearance(
        mut self,
        appearance: Option<Arc<dyn crate::local_seams::UnitAppearance>>,
    ) -> Self {
        self.appearance = Some(appearance);
        self
    }
    pub fn seed(mut self, seed: u32) -> Self {
        self.seed = seed;
        self
    }
    pub fn character(mut self, character: Character) -> Self {
        self.character = character;
        self
    }
    pub fn rules(mut self, rules: Ruleset) -> Self {
        self.rules = rules;
        self
    }
    /// Construct the simulation before its first client joins.
    pub fn build(self) -> Result<Sim, BuildError> {
        let appearance = self.appearance.unwrap_or_else(|| {
            let GameData::Live(d) = &self.data;
            super::appearance::AnimationNames::load(d.archives.as_ref())
                .ok()
                .map(|names| Arc::new(names) as Arc<dyn crate::local_seams::UnitAppearance>)
        });
        build_game(
            &self.data,
            self.seed,
            self.character,
            self.rules,
            appearance,
        )
    }
}
