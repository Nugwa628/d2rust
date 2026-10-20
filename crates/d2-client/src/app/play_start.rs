//! `play`'s game start: the command line (`--save`, `--new`, `--difficulty`,
//! `--hardcore`, `--save-dir`) or the front end's choice
//! ([`StartChoice`]) resolved into the character the join loads, the file
//! it is saved to and the 0x67 flags. `main` runs [`resolve`] before the
//! game window opens, so a bad folder or a taken name stops there. No game
//! logic.
//!
//! `// D2RUST's own, unverified` (decision D3: the CLI shortcuts are stand-ins).

use std::path::{Path, PathBuf};

use crate::app::front_start::StartChoice;
use crate::app::save;
use crate::app::single_player::{self, Character, GameData};

/// The command line's game-start options.
#[derive(Clone, Debug, Default)]
pub struct CliStart {
    /// `--save FILE`.
    pub save: Option<PathBuf>,
    /// `--new CLASS NAME`.
    pub new: Option<(String, String)>,
    /// `--save-dir DIR`.
    pub save_dir: Option<PathBuf>,
    /// `--difficulty`.
    pub difficulty: u8,
    /// `--hardcore`.
    pub hardcore: bool,
}

/// What the game window starts with.
#[derive(Debug)]
pub struct GameStart {
    pub character: Character,
    /// Where the character is written on exit (`save::save_path`).
    pub save_path: Option<PathBuf>,
    pub difficulty: u8,
    /// The front end's 0x67 flags; `None`: the character's own.
    pub start_flags: Option<u32>,
    pub hardcore: bool,
    /// Where the character came from, for the log line.
    pub origin: String,
}

/// Resolves the game start: the front end's `choice` when it made one,
/// else the command line. `menu_difficulty`: the difficulty popup's pick.
pub fn resolve(
    cli: &CliStart,
    data: &GameData,
    game_dir: Option<&Path>,
    menu_difficulty: Option<u8>,
    choice: Option<&StartChoice>,
) -> anyhow::Result<GameStart> {
    let difficulty = menu_difficulty.unwrap_or(cli.difficulty);
    // A front-end character is written to its own file: a new one's stub
    // (written by the create screen) is that file, not a taken name.
    let (save, new_name) = match choice {
        Some(c) => (Some(c.file.as_path()), None),
        None => (
            cli.save.as_deref(),
            cli.new.as_ref().map(|(_, name)| name.as_str()),
        ),
    };
    let save_path = save::save_path(save, new_name, cli.save_dir.as_deref(), game_dir)?;
    let (character, origin) = match (choice, &cli.save, &cli.new) {
        (Some(c), _, _) => (
            c.character(data)?,
            format!("front-end character {}", c.name),
        ),
        (None, Some(path), _) => (
            single_player::load_character(data, path, cli.difficulty)?,
            format!("character from {}", path.display()),
        ),
        (None, None, Some((class, name))) => (
            single_player::new_character(class, name)?,
            format!("new character {name} ({class})"),
        ),
        (None, None, None) => (Character::New, "default character".to_owned()),
    };
    Ok(GameStart {
        character: character.with_difficulty(difficulty),
        save_path,
        difficulty,
        start_flags: choice.map(|c| c.create_request().flags),
        hardcore: cli.hardcore || choice.is_some_and(StartChoice::hardcore),
        origin,
    })
}
