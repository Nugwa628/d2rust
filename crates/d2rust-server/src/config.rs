//! The server's settings, read from its config file.
//!
//! The file has one `key = value` per line. Blank lines and lines that
//! start with `#` are skipped. Every key is optional except `game_data`.
//! A relative path is taken from the config file's folder, so a server
//! folder can be moved as a whole.
//!
//! ```text
//! # d2rust-server.cfg
//! listen      = 0.0.0.0:6112
//! max_players = 8
//! game_data   = C:/Games/D2RUST/game
//! characters  = characters
//! logs        = logs
//! grace_s     = 30
//! frame_ms    = 1
//! max_games   = 16
//! save_interval_s = 60
//! server_name = D2RUST Europe
//! identity    = identity
//! max_sessions = 256
//! net_threads = 2
//! backups     = backups
//! backup_keep = 14
//! ```
//!
//! `dev = test` is for test servers only (the MP test tool): the server
//! takes dev commands on its standard input. The server logs a warning when
//! it is set.
//!
//! The accounts keys (`data`, `argon2`, `limits`, `max_characters`,
//! `registration`) are described at [`AccountsConfig`].

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use d2_server::adapters::session_flow::DEFAULT_MAX_PLAYERS;

use crate::accounts::AccountsConfig;

/// The highest player limit a game may be configured with. Nothing in the
/// game has a fixed player count; this bound is where per-tick costs have
/// been measured (64 scripted clients).
pub const MAX_PLAYERS_LIMIT: usize = 64;

/// The default listen address: every interface, port 6112.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:6112";

/// The default reconnect grace period, in seconds.
pub const DEFAULT_GRACE_S: u32 = 30;

/// The default number of games at once.
pub const DEFAULT_MAX_GAMES: usize = 16;

/// The default time between two saves of a game's players, in seconds.
pub const DEFAULT_SAVE_INTERVAL_S: u32 = 60;

/// The default number of sessions at once.
pub const DEFAULT_MAX_SESSIONS: usize = 256;

/// The default number of network threads.
pub const DEFAULT_NET_THREADS: usize = 2;

/// The default number of dated backups kept.
pub const DEFAULT_BACKUP_KEEP: usize = 14;

/// The default server name.
pub const DEFAULT_SERVER_NAME: &str = "D2RUST";

/// The server's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfig {
    /// The address and UDP port the QUIC endpoint listens on.
    pub listen: SocketAddr,
    /// The most players one game takes.
    pub max_players: usize,
    /// The game folder the server reads its game data from. Nothing
    /// from it is ever sent to clients.
    pub game_data: PathBuf,
    /// The character folder.
    pub characters: PathBuf,
    /// The log folder.
    pub logs: PathBuf,
    /// How long a lost player's slot is held for a reconnect.
    pub grace: Duration,
    /// The pause between two server frames.
    pub frame_pause: Duration,
    /// Accounts and online characters: where they are stored and their
    /// rules.
    pub accounts: AccountsConfig,
    /// The most games at once.
    pub max_games: usize,
    /// How often a game saves its players' characters.
    pub save_interval: Duration,
    /// The config file's folder: the server folder (`run/` is there).
    pub folder: PathBuf,
    /// The name clients see in the server's welcome.
    pub server_name: String,
    /// The folder of the server's certificate and key.
    pub identity: PathBuf,
    /// The most sessions at once.
    pub max_sessions: usize,
    /// The network side's threads.
    pub net_threads: usize,
    /// The folder of the dated backups.
    pub backups: PathBuf,
    /// How many dated backups are kept.
    pub backup_keep: usize,
    /// Test servers only: dev commands on standard input.
    pub dev_test: bool,
}

/// A config file that can't be used.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The file couldn't be read.
    #[error("reading {path}: {reason}")]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        reason: String,
    },
    /// A line is not `key = value`.
    #[error("line {line}: expected `key = value`")]
    Syntax {
        /// The line number, from 1.
        line: usize,
    },
    /// A key the server doesn't know.
    #[error("line {line}: unknown key `{key}`")]
    UnknownKey {
        /// The line number, from 1.
        line: usize,
        /// The key.
        key: String,
    },
    /// A key given twice.
    #[error("line {line}: `{key}` is already set")]
    Repeated {
        /// The line number, from 1.
        line: usize,
        /// The key.
        key: String,
    },
    /// A value that doesn't fit its key.
    #[error("line {line}: bad `{key}`: {reason}")]
    Value {
        /// The line number, from 1.
        line: usize,
        /// The key.
        key: String,
        /// Why.
        reason: String,
    },
    /// `game_data` is missing.
    #[error("`game_data` is required: the server reads the rules from the game folder")]
    NoGameData,
}

impl ServerConfig {
    /// Reads and parses the config file at `path`.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Read {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        let base = path.parent().unwrap_or(Path::new(""));
        Self::parse(&text, base)
    }

    /// Parses config text; relative paths are taken from `base`.
    pub fn parse(text: &str, base: &Path) -> Result<Self, ConfigError> {
        let mut listen = None;
        let mut max_players = None;
        let mut game_data = None;
        let mut characters = None;
        let mut logs = None;
        let mut grace_s = None;
        let mut frame_ms = None;
        let mut accounts = AccountsConfig::new(base);
        let mut accounts_keys: Vec<&str> = Vec::new();
        let mut max_games = None;
        let mut save_interval_s = None;
        let mut server_name = None;
        let mut identity = None;
        let mut max_sessions = None;
        let mut net_threads = None;
        let mut backups = None;
        let mut backup_keep = None;
        let mut dev_test = None;
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (key, value) = trimmed
                .split_once('=')
                .map(|(k, v)| (k.trim(), v.trim()))
                .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                .ok_or(ConfigError::Syntax { line })?;
            let bad = |reason: String| ConfigError::Value {
                line,
                key: key.into(),
                reason,
            };
            let repeated = || ConfigError::Repeated {
                line,
                key: key.into(),
            };
            match key {
                "listen" => {
                    let v = value
                        .parse::<SocketAddr>()
                        .map_err(|e| bad(e.to_string()))?;
                    set(&mut listen, v).ok_or_else(repeated)?;
                }
                "max_players" => {
                    let v = value.parse::<usize>().map_err(|e| bad(e.to_string()))?;
                    if !(1..=MAX_PLAYERS_LIMIT).contains(&v) {
                        return Err(bad(format!("must be 1 to {MAX_PLAYERS_LIMIT}")));
                    }
                    set(&mut max_players, v).ok_or_else(repeated)?;
                }
                "game_data" => set(&mut game_data, base.join(value)).ok_or_else(repeated)?,
                "characters" => set(&mut characters, base.join(value)).ok_or_else(repeated)?,
                "logs" => set(&mut logs, base.join(value)).ok_or_else(repeated)?,
                "grace_s" => {
                    let v = value.parse::<u32>().map_err(|e| bad(e.to_string()))?;
                    set(&mut grace_s, v).ok_or_else(repeated)?;
                }
                "frame_ms" => {
                    let v = value.parse::<u32>().map_err(|e| bad(e.to_string()))?;
                    if v > 1000 {
                        return Err(bad("must be 0 to 1000".into()));
                    }
                    set(&mut frame_ms, v).ok_or_else(repeated)?;
                }
                k if AccountsConfig::KEYS.contains(&k) => {
                    if accounts_keys.contains(&k) {
                        return Err(repeated());
                    }
                    accounts_keys.push(k);
                    accounts.set(k, value, base).map_err(bad)?;
                }
                "max_games" => {
                    let v = value.parse::<usize>().map_err(|e| bad(e.to_string()))?;
                    if !(1..=1024).contains(&v) {
                        return Err(bad("must be 1 to 1024".into()));
                    }
                    set(&mut max_games, v).ok_or_else(repeated)?;
                }
                "server_name" => {
                    if value.len() > 64 || value.chars().any(char::is_control) {
                        return Err(bad("at most 64 printable bytes".into()));
                    }
                    set(&mut server_name, value.to_owned()).ok_or_else(repeated)?;
                }
                "identity" => set(&mut identity, base.join(value)).ok_or_else(repeated)?,
                "backups" => set(&mut backups, base.join(value)).ok_or_else(repeated)?,
                "max_sessions" => {
                    let v = value.parse::<usize>().map_err(|e| bad(e.to_string()))?;
                    if !(1..=4096).contains(&v) {
                        return Err(bad("must be 1 to 4096".into()));
                    }
                    set(&mut max_sessions, v).ok_or_else(repeated)?;
                }
                "net_threads" => {
                    let v = value.parse::<usize>().map_err(|e| bad(e.to_string()))?;
                    if !(1..=16).contains(&v) {
                        return Err(bad("must be 1 to 16".into()));
                    }
                    set(&mut net_threads, v).ok_or_else(repeated)?;
                }
                "backup_keep" => {
                    let v = value.parse::<usize>().map_err(|e| bad(e.to_string()))?;
                    if !(1..=365).contains(&v) {
                        return Err(bad("must be 1 to 365".into()));
                    }
                    set(&mut backup_keep, v).ok_or_else(repeated)?;
                }
                "dev" => {
                    let v = match value {
                        "test" => true,
                        "off" => false,
                        _ => return Err(bad("`test` or `off`".into())),
                    };
                    set(&mut dev_test, v).ok_or_else(repeated)?;
                }
                "save_interval_s" => {
                    let v = value.parse::<u32>().map_err(|e| bad(e.to_string()))?;
                    if !(1..=3600).contains(&v) {
                        return Err(bad("must be 1 to 3600".into()));
                    }
                    set(&mut save_interval_s, v).ok_or_else(repeated)?;
                }
                _ => {
                    return Err(ConfigError::UnknownKey {
                        line,
                        key: key.into(),
                    })
                }
            }
        }
        Ok(ServerConfig {
            listen: listen.unwrap_or_else(|| DEFAULT_LISTEN.parse().expect("a valid address")),
            max_players: max_players.unwrap_or(DEFAULT_MAX_PLAYERS),
            game_data: game_data.ok_or(ConfigError::NoGameData)?,
            characters: characters.unwrap_or_else(|| base.join("characters")),
            logs: logs.unwrap_or_else(|| base.join("logs")),
            grace: Duration::from_secs(grace_s.unwrap_or(DEFAULT_GRACE_S).into()),
            frame_pause: Duration::from_millis(frame_ms.unwrap_or(1).into()),
            accounts,
            max_games: max_games.unwrap_or(DEFAULT_MAX_GAMES),
            save_interval: Duration::from_secs(
                save_interval_s.unwrap_or(DEFAULT_SAVE_INTERVAL_S).into(),
            ),
            folder: base.to_path_buf(),
            server_name: server_name.unwrap_or_else(|| DEFAULT_SERVER_NAME.to_owned()),
            identity: identity.unwrap_or_else(|| base.join("identity")),
            max_sessions: max_sessions.unwrap_or(DEFAULT_MAX_SESSIONS),
            net_threads: net_threads.unwrap_or(DEFAULT_NET_THREADS),
            backups: backups.unwrap_or_else(|| base.join("backups")),
            backup_keep: backup_keep.unwrap_or(DEFAULT_BACKUP_KEEP),
            dev_test: dev_test.unwrap_or(false),
        })
    }
}

/// Sets `slot` to `v` if it is empty; `None` if it was already set.
fn set<T>(slot: &mut Option<T>, v: T) -> Option<()> {
    if slot.is_some() {
        return None;
    }
    *slot = Some(v);
    Some(())
}
