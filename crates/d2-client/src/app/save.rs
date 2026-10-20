//! Saving the played character (stitch-save; `d2-client play`).
//!
//! The server writes the player's `.d2s`: the
//! app installs a [`FileStore`] as the server's character storage, and
//! the server runs it in the leave of C→S 0x69 (Save and Exit,
//! [`request_save_and_exit`]) before its 0x05, and every 8192 frames.
//! Where it goes: `play --save <file>` writes that file;
//! `play --new <class> <name>` writes `<name>.d2s` in a D2RUST save
//! folder ([`default_save_dir`], or `--save-dir`; never inside the game
//! install).
//!
//! What a save holds today (D2RUST's own, unverified): the character the
//! join loaded (a parsed save, or a fresh header for `--new`) with the
//! live values the load restores overlaid: the 16 base stats
//! ([`SNAPSHOT_STATS`]; `ActionCharacter::set_base_stat` loads them
//! back) and the quest flag records (`rest.quests`). Everything the sim
//! cannot yet hold or read back (skills, waypoints, items, mouse
//! skills, hotkeys, NPC fields: `ActionCharacter`'s unapplied steps)
//! passes through from the loaded save unchanged, and is empty for a new
//! character. The file's bytes are the `d2_formats::d2s::write` writer's.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy::app::AppExit;
use bevy::prelude::{MessageWriter, Resource};
use d2_formats::d2s::{self, D2s, SaveTables};

use d2_formats::d2s::appearance::AppearanceTables;
use d2_server::adapters::storage::CharacterStore;
use d2_server::game::character_save;
pub use d2_server::game::character_save::{
    appearance_tables, apply_live, base_save, old_items_pass_through, Live, NpcField,
    SNAPSHOT_STATS,
};
use d2_server::seams::ClientId;
use d2_sim::wiring::worldgen::WorldSim;

use super::server_thread::ThreadLink;
use super::single_player::{self, CharacterCodec, D2sCodec, Link, LocalSeams, Sim, World};
use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};

/// Why a save did not happen.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("{0}")]
    Path(String),
    #[error("no joined player to save")]
    NoPlayer,
    #[error("server thread: {0}")]
    Server(String),
    #[error("writing the save: {0}")]
    Format(String),
    #[error("{path}: {message}")]
    Io { path: String, message: String },
}

/// The save folder: `saves` in the player's D2RUST folder
/// (`Documents\D2RUST\saves`, [`crate::user_dir`]).
pub fn default_save_dir() -> PathBuf {
    crate::user_dir::saves()
}

/// Where the character is written: `--save <file>` is that file;
/// `--new <class> <name>` is `<name>.d2s` in `save_dir` (default
/// [`default_save_dir`]). `None`: neither flag, so the default character
/// is not saved. A folder inside `game_dir` (the install) is refused, and
/// so is a `--new` file that already exists (it would overwrite a
/// character; load it with `--save`).
// D2RUST's own, unverified
pub fn save_path(
    save: Option<&Path>,
    new: Option<&str>,
    save_dir: Option<&Path>,
    game_dir: Option<&Path>,
) -> Result<Option<PathBuf>, SaveError> {
    let inside = |p: &Path| game_dir.is_some_and(|g| resolved(p).starts_with(resolved(g)));
    let path = match (save, new) {
        (Some(file), _) => file.to_path_buf(),
        (None, Some(name)) => {
            let dir = save_dir.map_or_else(default_save_dir, Path::to_path_buf);
            let file = dir.join(format!("{name}.d2s"));
            if file.exists() {
                return Err(SaveError::Path(format!(
                    "{} already exists: load it with --save, or pick another name",
                    file.display()
                )));
            }
            file
        }
        (None, None) => return Ok(None),
    };
    if inside(&path) {
        return Err(SaveError::Path(format!(
            "{} is inside the game install: saves go elsewhere (--save-dir)",
            path.display()
        )));
    }
    Ok(Some(path))
}

/// `p` with its longest existing ancestor canonicalized and the rest
/// (not yet created: the save file, a new save folder) joined back on,
/// so two paths compare in the same form: on Windows `canonicalize`
/// gives a `\\?\C:\...` path, which never `starts_with` a plain one.
/// `p` as it is when no ancestor resolves (or a `..` is in the tail).
// D2RUST's own, unverified
fn resolved(p: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut cur = p;
    loop {
        if let Ok(mut out) = std::fs::canonicalize(cur) {
            out.extend(tail.iter().rev());
            return out;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name);
                cur = if parent.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    parent
                };
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Reads [`Live`] from the game's local player
/// ([`character_save::read_live`]).
pub fn read_live(sim: &mut Sim) -> Result<Live, SaveError> {
    let (player, _) = single_player::local_player(sim).ok_or(SaveError::NoPlayer)?;
    Ok(character_save::read_live(sim, player))
}

/// Writes `save` to `path`: to a temporary file next to it, then renamed
/// over it; an existing file is first copied to `<path>.bak`.
pub fn write_file(path: &Path, save: &D2s, tables: &dyn SaveTables) -> Result<(), SaveError> {
    write_file_with(path, save, tables, &D2sCodec)
}

/// [`write_file`] in the game's own character format (`codec`: a mod
/// game's, through its [`single_player::Saves`]).
pub fn write_file_with(
    path: &Path,
    save: &D2s,
    tables: &dyn SaveTables,
    codec: &dyn CharacterCodec,
) -> Result<(), SaveError> {
    let io = |e: std::io::Error| SaveError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    };
    let bytes = codec.write(save, tables).map_err(SaveError::Format)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    if path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        std::fs::copy(path, PathBuf::from(bak)).map_err(io)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// A [`ThreadLink`] the app and the [`SaveHandle`] both hold: every call
/// goes through the lock, in the caller's order.
pub struct SharedLink<C: d2_server::seams::Clock + Send + 'static>(Arc<Mutex<ThreadLink<Link<C>>>>);

impl<C: d2_server::seams::Clock + Send + 'static> SharedLink<C> {
    /// Shares `link` with no save handle (a game that is not saved).
    pub fn new(link: ThreadLink<Link<C>>) -> Self {
        Self(Arc::new(Mutex::new(link)))
    }

    /// The shared server thread itself, for tools that read the game
    /// between frames (`play --control`).
    pub fn server(&self) -> Arc<Mutex<ThreadLink<Link<C>>>> {
        self.0.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ThreadLink<Link<C>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs `f` on the server thread's link between frames (tests, tools).
    pub fn with<R, F>(&self, f: F) -> Result<R, super::server_thread::ThreadStopped>
    where
        R: Send + 'static,
        F: FnOnce(&mut Link<C>) -> R + Send + 'static,
    {
        self.lock().with(f)
    }
}

impl<C: d2_server::seams::Clock + Send + 'static> Clone for SharedLink<C> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<C: d2_server::seams::Clock + Send + 'static> ServerLink for SharedLink<C> {
    fn protocol_version(&self) -> u32 {
        self.lock().protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.lock().tick_rate()
    }
    fn server_frame(&mut self) -> Option<i32> {
        self.lock().server_frame()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.lock().send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.lock().pump()
    }
    fn set_display_catch_up(&mut self, enabled: bool) {
        self.lock().set_display_catch_up(enabled)
    }
    fn tick_pending(&mut self) -> bool {
        self.lock().tick_pending()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.lock().receive()
    }
    fn server_paths(&mut self) -> Vec<crate::bridge::critter_path::ServerPath> {
        server_paths_on(&mut self.lock())
    }
    fn set_send_now(&mut self, now: Option<u32>) {
        self.lock().set_send_now(now)
    }
}

/// The single-player server's paths, read on its thread for the client's
/// server copy and its own path step (`ServerLink::server_paths`). Every
/// link that holds the in-process game answers with this read, so a
/// scenario, `autoplay-host` and `play --control` run the client update the
/// way a saved game's `play` does (a knockback or a moving skill ends when
/// the server's path does). A stopped thread has none.
pub fn server_paths_on<C: d2_server::seams::Clock + Send + 'static>(
    link: &mut ThreadLink<Link<C>>,
) -> Vec<crate::bridge::critter_path::ServerPath> {
    let snap = link.with(|l| {
        let h = l.host();
        let dirs = d2_sim::debug::state::player_new_directions_world(&h.game.game, &h.game.events);
        (super::state_dump::snapshot_host(h), dirs)
    });
    snap.map(|(s, dirs)| {
        let mut paths = super::state_dump::server_paths_of(&s);
        for p in paths.iter_mut().filter(|p| p.key.unit_type == 0) {
            p.new_dir = dirs.iter().find(|(g, _)| *g == p.key.guid).map(|(_, d)| *d);
        }
        paths
    })
    .unwrap_or_default()
}

/// The server's character writer of the app's game (`d2-server`
/// `adapters::storage`): the server runs it when the player saves and
/// exits (C→S 0x69, before S→C 0x05) and every 8192 frames; it reads the
/// running game ([`read_live`]), lays it over `base` ([`apply_live`]) and
/// writes `path` in the game's character format ([`write_file_with`]).
pub struct FileStore {
    pub path: PathBuf,
    pub base: D2s,
    pub tables: Arc<dyn SaveTables + Send + Sync>,
    /// The appearance tables of the game's data ([`appearance_tables`]):
    /// with them a save rebuilds the header's appearance bytes from the
    /// equipped items; `None` (no tables): the loaded bytes stay.
    pub appearance: Option<Arc<AppearanceTables>>,
    /// The game's character format (D2RUST's: [`D2sCodec`]).
    pub codec: Arc<dyn CharacterCodec>,
}

impl CharacterStore<WorldSim<LocalSeams>, World> for FileStore {
    /// Saves the local player's character to its file. The store holds
    /// one local character: another client's player (a game with several
    /// clients) is not written here; online characters are kept by the
    /// server's own store.
    fn save(&mut self, sim: &mut Sim, client: ClientId) -> Result<(), String> {
        let local = single_player::local_player(sim).map(|(p, _)| p);
        if local.is_none() || sim.player_of(client) != local {
            return Ok(());
        }
        let live = read_live(sim).map_err(|e| e.to_string())?;
        // A 0x5C–0x5E file whose item records would pass through as
        // loaded is not overwritten (the file and its `.bak` stay).
        if self.base.header.version < d2s::VERSION && old_items_pass_through(&self.base, &live) {
            return Err(format!(
                "{}: a version {:#x} save whose items the game cannot rewrite is not overwritten",
                self.path.display(),
                self.base.header.version
            ));
        }
        let mut save = apply_live(&self.base, &live, character_save::now_secs());
        if let (Some(t), Some((player, _))) = (&self.appearance, single_player::local_player(sim)) {
            character_save::rebuild_appearance(sim, player, t, &mut save);
        }
        write_file_with(&self.path, &save, &*self.tables, self.codec.as_ref())
            .map_err(|e| e.to_string())
    }
}

/// A save the app asks of the running game (the death saves,
/// [`super::hardcore`]): the server's save call with its installed
/// [`FileStore`], run on the server thread between frames. Cloning
/// shares the same game and file.
#[derive(Clone, Resource)]
pub struct SaveHandle {
    path: PathBuf,
    save: Arc<dyn Fn() -> Result<(), SaveError> + Send + Sync>,
}

impl SaveHandle {
    /// The file a save writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the character now.
    pub fn save(&self) -> Result<(), SaveError> {
        (self.save)()
    }
}

/// Installs a [`FileStore`] that writes `path` with `base`
/// ([`base_save`]) and the `tables` of the game as the server's character
/// storage, and shares `link` with a [`SaveHandle`] over it. The file is
/// a `.d2s` ([`share_with`] for a game's own format).
pub fn share<C: d2_server::seams::Clock + Send + 'static>(
    link: ThreadLink<Link<C>>,
    base: D2s,
    tables: Arc<dyn SaveTables + Send + Sync>,
    appearance: Option<Arc<AppearanceTables>>,
    path: PathBuf,
) -> Result<(SharedLink<C>, SaveHandle), SaveError> {
    share_with(link, base, tables, appearance, path, Arc::new(D2sCodec))
}

/// [`share`] writing the game's own character format (`codec`, from the
/// game's [`single_player::Saves`]).
pub fn share_with<C: d2_server::seams::Clock + Send + 'static>(
    mut link: ThreadLink<Link<C>>,
    base: D2s,
    tables: Arc<dyn SaveTables + Send + Sync>,
    appearance: Option<Arc<AppearanceTables>>,
    path: PathBuf,
    codec: Arc<dyn CharacterCodec>,
) -> Result<(SharedLink<C>, SaveHandle), SaveError> {
    let store = FileStore {
        path: path.clone(),
        base,
        tables,
        appearance,
        codec,
    };
    link.with(move |l| l.host_mut().game.set_storage(Box::new(store)))
        .map_err(|e| SaveError::Server(e.to_string()))?;
    let shared = Arc::new(Mutex::new(link));
    let handle = shared.clone();
    let save = move || {
        let mut link = handle.lock().unwrap_or_else(|e| e.into_inner());
        let saved = link
            .with(|l| l.host_mut().game.save_characters())
            .map_err(|e| SaveError::Server(e.to_string()))?;
        match saved.into_iter().next() {
            None => Err(SaveError::NoPlayer),
            Some((_, r)) => r.map_err(|e| SaveError::Server(e.to_string())),
        }
    };
    Ok((
        SharedLink(shared),
        SaveHandle {
            path,
            save: Arc::new(save),
        },
    ))
}

/// The Esc game menu's Save and Exit Game:
/// C→S 0x69 on the system queue and the model's `exit_requested`. The
/// server saves and answers 0x05, 0x06; [`end_of_game`] then closes the
/// app.
pub fn request_save_and_exit<L: ServerLink>(
    bridge: &mut crate::bridge::Bridge<L>,
) -> Result<(), crate::bridge::BridgeError> {
    bridge.save_and_exit().map(|_| ())
}

/// The client's end of the game: the server's 0x05 took the client out of the
/// game (`in_game` false) and the exit is asked (0x06, or the Save and Exit
/// send): the app closes. The game returns to the main menu;
/// `main.rs`'s `play` loop reopens the front end at `Entry::MainMenu`, and this
/// system only ends the app for the headless runs.
pub fn end_of_game(
    bridge: Option<bevy::prelude::Res<crate::bridge::BridgeResource>>,
    one_app: Option<bevy::prelude::Res<super::session::OneApp>>,
    mut exit: MessageWriter<AppExit>,
    mut done: bevy::prelude::Local<bool>,
) {
    let Some(b) = bridge else {
        return;
    };
    if one_app.is_some() {
        // The session goes back to the front end (`session::leave_to_menu`).
        return;
    }
    if bevy::prelude::DetectChanges::is_added(&b) {
        *done = false;
    }
    let w = b.0.world();
    if !*done && w.exit_requested && !w.in_game {
        *done = true;
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first save of a character created on the menu starts from the
    /// header-only stub; the saved file has a body, so it must not keep the
    /// stub's "new" flag (the loader refuses that flag with a body).
    #[test]
    fn first_save_of_a_new_character_drops_the_new_flag() {
        let stub = D2s::new_stub(b"qwer", 1, 0, 1).expect("stub");
        assert_ne!(stub.header.status & d2s::status::NEW, 0);
        let saved = apply_live(&stub, &Live::default(), 2);
        assert!(saved.body.is_some());
        assert_eq!(saved.header.status & d2s::status::NEW, 0);
    }
}
