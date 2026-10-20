//! App save defaults and local-link startup over the headless game builder.
use std::path::{Path, PathBuf};

use d2_formats::d2s::SaveTables;

use super::*;
use crate::app::save::{write_file_with, SaveError};

/// Where a game keeps its characters and in which format.
#[derive(Debug, Clone)]
pub struct Saves {
    /// The game's save folder.
    pub folder: PathBuf,
    /// The game's character format.
    pub codec: Arc<dyn CharacterCodec>,
}

impl Saves {
    /// A game's own save folder and format.
    pub fn new(folder: impl Into<PathBuf>, codec: Arc<dyn CharacterCodec>) -> Saves {
        Saves {
            folder: folder.into(),
            codec,
        }
    }

    /// D2RUST's saves: `.d2s` files in the `saves` folder of the player's
    /// D2RUST folder ([`crate::user_dir::saves`]).
    pub fn d2rust() -> Saves {
        Saves::new(crate::user_dir::saves(), Arc::new(D2sCodec))
    }

    /// The save file of the character `name` (`<folder>/<name>.<ext>`).
    pub fn file(&self, name: &str) -> PathBuf {
        self.folder
            .join(format!("{name}.{}", self.codec.extension()))
    }

    /// The character of the save at `path`, read with the game's codec,
    /// loaded on `difficulty` ([`load_character`] for the `.d2s` codec).
    pub fn load(
        &self,
        data: &GameData,
        path: &Path,
        difficulty: u8,
    ) -> Result<Character, BuildError> {
        load_character_with(data, path, difficulty, self.codec.as_ref())
    }

    /// Writes `save` to `path` with the game's codec (the previous file
    /// kept as `.bak`, the write through a `.tmp` file).
    pub fn write(&self, path: &Path, save: &D2s, tables: &dyn SaveTables) -> Result<(), SaveError> {
        write_file_with(path, save, tables, self.codec.as_ref())
    }
}

/// A single-player game to build: the loaded data and everything the game
/// chooses. [`GameBuilder::new`] starts from D2RUST's defaults; a mod
/// game's binary sets its own rules and saves.
#[derive(Debug, Clone)]
pub struct GameBuilder {
    /// The player's tables and level data, loaded with the game's layers
    /// ([`GameData::open`]).
    pub data: GameData,
    /// The game seed (`--seed`; [`DEFAULT_SEED`] by default).
    pub seed: u32,
    /// The character the join loads ([`Character::New`] by default).
    pub character: Character,
    /// The game's rules, set once at creation (`Ruleset::Original` by
    /// default; only a mod game passes `Ruleset::Mod`).
    pub rules: Ruleset,
    /// The game's save folder and character format ([`Saves::d2rust`] by
    /// default).
    pub saves: Saves,
}

impl GameBuilder {
    /// A game on `data` with D2RUST's defaults.
    pub fn new(data: GameData) -> GameBuilder {
        GameBuilder {
            data,
            seed: DEFAULT_SEED,
            character: Character::New,
            rules: Ruleset::Original,
            saves: Saves::d2rust(),
        }
    }

    /// The game seed.
    pub fn seed(mut self, seed: u32) -> GameBuilder {
        self.seed = seed;
        self
    }

    /// The character the join loads.
    pub fn character(mut self, character: Character) -> GameBuilder {
        self.character = character;
        self
    }

    /// The game's rules.
    pub fn rules(mut self, rules: Ruleset) -> GameBuilder {
        self.rules = rules;
        self
    }

    /// The game's save folder and character format.
    pub fn saves(mut self, saves: Saves) -> GameBuilder {
        self.saves = saves;
        self
    }

    /// Builds the game on this thread ([`build`] describes what is built).
    pub fn build(self) -> Result<LocalGame, BuildError> {
        let mut game = build_game(&self.data, self.seed, self.character, self.rules)?;
        game.saves = self.saves;
        Ok(game)
    }

    /// Builds the game on a new server thread behind a local host with
    /// `clock` ([`start`]). The game's [`Saves`] stay with the caller:
    /// clone [`GameBuilder::saves`] before starting.
    pub fn start<C: Clock + Send + 'static>(
        self,
        clock: C,
    ) -> Result<(ThreadLink<Link<C>>, Started), BuildError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let link = ThreadLink::spawn(move || {
            // A mod game's rules speak the mod wire: its envelopes carry
            // the mod game's own messages (MS13).
            let wire = if self.rules.is_original() {
                Wire::Original
            } else {
                Wire::Mod
            };
            let g = self.build()?;
            let _ = tx.send(Started {
                prices: g.sim.world.rest.prices.clone(),
                wire,
            });
            let mut host = Host::new(g.sim, ProtoSizes, PendingSession::default(), clock);
            host.set_wire(wire);
            Ok::<_, BuildError>(LocalLink::new(host))
        })?;
        let started = rx
            .recv()
            .map_err(|_| BuildError::Setup("game not started".into()))?;
        Ok((link, started))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mod game's own format: a 4-byte tag in front of the `.d2s` bytes.
    #[derive(Debug)]
    struct Tagged;

    /// No item or stat tables: the test codec never reads them.
    struct NoTables;

    impl SaveTables for NoTables {
        fn stat_save(&self, _: u16) -> Option<d2_formats::d2s::StatSave> {
            None
        }
        fn item_entry_len(&self, _: &[u8]) -> Result<usize, String> {
            Err("no items".into())
        }
    }

    impl CharacterCodec for Tagged {
        fn extension(&self) -> &str {
            "modsave"
        }
        fn read(&self, _: &LiveData, _: &[u8], _: u8) -> Result<D2s, String> {
            Err("not read in this test".into())
        }
        fn write(&self, _: &D2s, _: &dyn SaveTables) -> Result<Vec<u8>, String> {
            Ok(b"MODG".to_vec())
        }
    }

    #[test]
    fn layers_name_the_order_file_or_an_explicit_list() {
        assert_eq!(Layers::default(), Layers::OrderFile);
        assert_eq!(Layers::OrderFile.order(), None);
        assert_eq!(Layers::base().order(), Some(&[][..]));
        let list = Layers::List(vec!["a".into(), "b".into()]);
        assert_eq!(list.order().map(<[String]>::len), Some(2));
    }

    #[test]
    fn d2rust_saves_are_d2s_files_in_the_user_folder() {
        let s = Saves::d2rust();
        assert_eq!(s.folder, crate::user_dir::saves());
        assert_eq!(s.file("Conan"), crate::user_dir::saves().join("Conan.d2s"));
    }

    #[test]
    fn a_game_writes_its_own_format_into_its_own_folder() {
        let dir = std::env::temp_dir().join(format!("d2rust-ms2-saves-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let saves = Saves::new(dir.join("modgame"), Arc::new(Tagged));
        let path = saves.file("Hero");
        assert_eq!(path, dir.join("modgame").join("Hero.modsave"));
        let save = crate::app::save::base_save(&Character::New);
        saves.write(&path, &save, &NoTables).expect("written");
        assert_eq!(std::fs::read(&path).unwrap(), b"MODG");
        // A second write keeps the first as `.bak`.
        saves.write(&path, &save, &NoTables).unwrap();
        assert!(dir.join("modgame").join("Hero.modsave.bak").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
