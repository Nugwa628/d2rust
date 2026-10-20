//! Online play (the `online` feature): the server list, and a game on a
//! remote server.
//!
//! The launcher writes `servers.toml` next to the game exe from its signed
//! manifest before it starts the game for online play:
//!
//! ```toml
//! [[server]]
//! id = "eu1"
//! name = "D2RUST Europe"
//! game = "d2rust"
//! address = "37.27.64.176:6112"
//! cert_sha256 = "<64 hex>"
//! version = "0.1.2.0"
//! ```
//!
//! The game pins each server's certificate to that fingerprint, so a server
//! the list doesn't name can't stand in. Without the file (the game started
//! on its own, e.g. from the release zip) the game uses its own list, the
//! D2RUST server ([`ServerList::builtin`]). The online screens (login,
//! characters, lobby) use a [`d2_online::Session`]; a joined game runs on
//! the bridge's remote link ([`crate::bridge::remote`]) with the same live
//! client single player runs.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

use anyhow::Context;
use bevy::prelude::*;
use d2_net::wire::ResumeToken;
use d2_net::NetError;
use d2_online::{
    parse_fingerprint, ClientInfo, GameLink, OnlineError, ServerSpec, ServerTrust, Session,
};
use d2_sim::tick::TickRate;

use super::play::{self, LiveClient, PlayHandles};
use super::session::{Screen, Setup, ToMenu};
use super::single_player::{self, Character, GameData, LiveData, NewCharacter};
use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use crate::bridge::mirror::DynLink;
use crate::bridge::remote::{Connection, Reconnect, RemoteLink, RemoteStatus};

/// The file the launcher writes next to the game exe.
pub const SERVERS_FILE: &str = "servers.toml";

/// What the online screens say when there is no server list.
pub const NO_LIST: &str = "Start the game once from the launcher to get the server list.";

/// The D2RUST server, for [`ServerList::builtin`]: where it listens and the
/// SHA-256 of its certificate (the server prints both in its `ready` line).
pub const DEFAULT_SERVER_ID: &str = "eu1";
pub const DEFAULT_SERVER_NAME: &str = "D2RUST Europe";
pub const DEFAULT_SERVER_ADDRESS: &str = "37.27.64.176:6112";
pub const DEFAULT_SERVER_CERT: &str =
    "8bf0c9ed67a2eae0f4684accbfa464cf612bc530ec7f468401883a8446771e67";

/// One server of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerEntry {
    pub id: String,
    pub name: String,
    /// `d2rust`, or a mod game's id.
    pub game: String,
    /// `host:port`.
    pub address: String,
    pub trust: ServerTrust,
    /// The game version the server runs.
    pub version: String,
}

impl ServerEntry {
    /// The entry as the session connects to it.
    pub fn spec(&self) -> ServerSpec {
        ServerSpec {
            id: self.id.clone(),
            name: self.name.clone(),
            game: self.game.clone(),
            address: self.address.clone(),
            trust: self.trust.clone(),
            version: self.version.clone(),
        }
    }
}

/// The servers the launcher listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerList {
    pub servers: Vec<ServerEntry>,
}

impl ServerList {
    /// `servers.toml` next to the running exe.
    pub fn path() -> Option<PathBuf> {
        Some(std::env::current_exe().ok()?.parent()?.join(SERVERS_FILE))
    }

    /// The list next to the running exe, else [`Self::builtin`]; the error
    /// (a damaged list) is the text the screens show.
    pub fn load() -> Result<Self, String> {
        Self::load_from(Self::path().as_deref())
    }

    /// The list in `path`; [`Self::builtin`] when there is no such file.
    pub fn load_from(path: Option<&Path>) -> Result<Self, String> {
        match path {
            Some(p) if p.is_file() => Self::read(p),
            _ => Ok(Self::builtin()),
        }
    }

    /// The game's own list: the D2RUST server, its certificate pinned. A
    /// release ships the game and the server from one commit, so the server
    /// runs this build's version. The launcher's list replaces it.
    pub fn builtin() -> Self {
        let fp = parse_fingerprint(DEFAULT_SERVER_CERT).expect("the built-in fingerprint is valid");
        Self {
            servers: vec![ServerEntry {
                id: DEFAULT_SERVER_ID.into(),
                name: DEFAULT_SERVER_NAME.into(),
                game: "d2rust".into(),
                address: DEFAULT_SERVER_ADDRESS.into(),
                trust: ServerTrust::Pinned(fp),
                version: crate::launch::VERSION.into(),
            }],
        }
    }

    /// The list in `path`.
    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|_| NO_LIST.to_owned())?;
        Self::parse(&text).map_err(|e| format!("The server list is damaged ({e}). {NO_LIST}"))
    }

    /// Parses the file's text. An entry missing a field or with a bad
    /// fingerprint fails the whole list: the launcher writes it whole.
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        let Some(item) = doc.get("server") else {
            return Ok(Self::default());
        };
        let tables = item
            .as_array_of_tables()
            .ok_or("`server` is not a list of tables")?;
        let mut servers = Vec::new();
        for (i, t) in tables.iter().enumerate() {
            let field = |k: &str| {
                t.get(k)
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .ok_or_else(|| format!("server {} has no `{k}`", i + 1))
            };
            let cert = field("cert_sha256")?;
            let fp = parse_fingerprint(&cert)
                .ok_or_else(|| format!("server {} has a bad `cert_sha256`", i + 1))?;
            servers.push(ServerEntry {
                id: field("id")?,
                name: field("name")?,
                game: field("game")?,
                address: field("address")?,
                trust: ServerTrust::Pinned(fp),
                version: field("version")?,
            });
        }
        Ok(Self { servers })
    }

    /// The server `id`, or the first one when `id` is `None`.
    pub fn pick(&self, id: Option<&str>) -> Option<&ServerEntry> {
        match id {
            Some(id) => self.servers.iter().find(|s| s.id == id),
            None => self.servers.first(),
        }
    }
}

/// What this build is, for the handshake: the server must run exactly the
/// same protocol, game, version and tick rate.
pub fn client_info() -> ClientInfo {
    ClientInfo {
        protocol: d2_proto::PROTOCOL_VERSION,
        game: "d2rust".into(),
        build: crate::launch::VERSION.into(),
        tick_rate: TickRate::GAME.hz(),
    }
}

/// The session with the server, shared by the online screens (which
/// connect and fill it) and the game (which joins through it). It stays
/// open across games: after a game the player is back in the lobby.
#[derive(Resource, Clone, Default)]
pub struct OnlineSession(pub Arc<Mutex<Option<Session>>>);

/// The game the lobby let the player join: its ticket and the selected
/// character, whose name and class the game's join (C→S 0x67) must carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnlineJoin {
    /// From the lobby's `JoinOk`.
    pub ticket: [u8; 16],
    pub name: String,
    /// Class id 0-6.
    pub class: u8,
    /// The character's save status word (hardcore, expansion, progression).
    pub status: u16,
    /// The game's difficulty 0-2.
    pub difficulty: u8,
}

impl OnlineJoin {
    /// The join the lobby made, once: its `JoinOk` (ticket, difficulty) and
    /// the selected character. `None`: no join, or no character selected.
    pub fn take(online: &mut crate::ui::front_end::screens::online::Online) -> Option<Self> {
        let c = online.state.selected.clone()?;
        let j = online.take_joined()?;
        Some(Self {
            ticket: j.ticket,
            name: c.name,
            class: c.class,
            status: c.status,
            difficulty: j.difficulty,
        })
    }
}

/// Why the last online game ended without the player leaving it, for the
/// online screens to show once. The host inserts it before the first game
/// (it outlives each game) and reads it when the front end opens again;
/// when the session is gone too ([`OnlineSession`] empty), the screens go
/// back to the login.
#[derive(Resource, Clone, Default)]
pub struct OnlineEnd(pub Arc<Mutex<Option<String>>>);

impl OnlineEnd {
    fn set(&self, why: String) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(why);
        }
    }

    /// The reason, once.
    pub fn take(&self) -> Option<String> {
        self.0.lock().ok().and_then(|mut s| s.take())
    }
}

/// The running online game's connection, for the watch and the UI.
#[derive(Resource, Clone)]
pub struct OnlineGame(pub RemoteStatus);

/// A link the bridge can hold (`Send + Sync`) around one that is only
/// `Send`: the bridge calls it through `&mut`, so the lock is never waited on.
struct SyncLink<L>(Mutex<L>);

impl<L: ServerLink + Send> SyncLink<L> {
    fn get(&mut self) -> &mut L {
        self.0.get_mut().unwrap_or_else(|p| p.into_inner())
    }
}

impl<L: ServerLink + Send> ServerLink for SyncLink<L> {
    fn protocol_version(&self) -> u32 {
        d2_proto::PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> TickRate {
        match self.0.lock() {
            Ok(l) => l.tick_rate(),
            Err(p) => p.into_inner().tick_rate(),
        }
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.get().send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.get().pump()
    }
    fn set_display_catch_up(&mut self, enabled: bool) {
        self.get().set_display_catch_up(enabled)
    }
    fn tick_pending(&mut self) -> bool {
        self.get().tick_pending()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.get().receive()
    }
    fn is_remote(&self) -> bool {
        true
    }
    fn take_resume(&mut self) -> bool {
        self.get().take_resume()
    }
}

/// The C→S 0x67 character of an online join: a character of the selected
/// name and class (the server plays its own stored record of it).
fn join_character(join: &OnlineJoin) -> anyhow::Result<Character> {
    let bytes = join.name.as_bytes();
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() < 16 && join.class < 7,
        "the character {:?} can't be played",
        join.name
    );
    let mut name = [0; 16];
    name[..bytes.len()].copy_from_slice(bytes);
    Ok(Character::Named(NewCharacter {
        class: join.class,
        name,
        difficulty: join.difficulty,
    }))
}

/// Joins the game of `join` through the session and installs it on `app`:
/// the bridge on the remote link, then the live client single player runs.
/// A broken connection is retried in the background while the server
/// holds the slot. The join waits for the server's answer; the session's
/// game start goes through [`start_online`], which waits on a worker thread.
pub fn install_online(
    app: &mut impl Setup,
    live: &Arc<LiveData>,
    session: &OnlineSession,
    join: &OnlineJoin,
) -> anyhow::Result<PlayHandles> {
    join_character(join)?;
    let game = join_game(session, join.ticket).map_err(|e| anyhow::anyhow!(e.message()))?;
    install_joined(app, live, session, join, game)
}

/// The game stream of the lobby's ticket (blocks until the server answers).
fn join_game(session: &OnlineSession, ticket: [u8; 16]) -> Result<GameLink, OnlineError> {
    let mut slot = session
        .0
        .lock()
        .map_err(|_| OnlineError::Lost(NetError::Closed))?;
    let s = slot.as_mut().ok_or(OnlineError::Lost(NetError::Closed))?;
    s.join(ticket)
}

/// Installs the joined `game` on `app` ([`install_online`]'s second half).
fn install_joined(
    app: &mut impl Setup,
    live: &Arc<LiveData>,
    session: &OnlineSession,
    join: &OnlineJoin,
    game: GameLink,
) -> anyhow::Result<PlayHandles> {
    let character = join_character(join)?;
    let rate = TickRate::new(game.welcome().tick_rate).context("the server's tick rate")?;
    let shared = session.0.clone();
    let hook: Reconnect<GameLink> = Box::new(move |token: Option<ResumeToken>| {
        // The link keeps the game's token and replaces it with the new
        // welcome's after every resume.
        let token = token.ok_or(OnlineError::Lost(NetError::Closed))?;
        let mut slot = shared
            .lock()
            .map_err(|_| OnlineError::Lost(NetError::Closed))?;
        let old = slot.as_ref().ok_or(OnlineError::Lost(NetError::Closed))?;
        let mut fresh = old.reconnect()?;
        let game = fresh.rejoin(token)?;
        *slot = Some(fresh);
        Ok(game)
    });
    let remote = RemoteLink::new(game, rate).with_reconnect(hook);
    let status = remote.status();
    let link: DynLink = Box::new(SyncLink(Mutex::new(remote)));

    let data = GameData::Live(live.clone());
    let speeds = single_player::walk_speeds(&data, &character)?;
    let cfg_dir = super::config::config_dir(&super::save::default_save_dir());
    let settings = super::config::load_settings(&cfg_dir)?;
    let bindings = super::config::load_controls(&super::config::controls_dir(
        crate::ui::front_end::screens::controls::config_path().as_deref(),
        &cfg_dir,
    ))?;
    app.world_mut().insert_resource(super::config::ConfigRes {
        dir: cfg_dir,
        settings,
        bindings,
        sound_device: true,
    });
    let hardcore = join.status & d2_formats::d2s::status::HARDCORE != 0;
    // The shop's board: the client prices what it shows (contracts 3.7
    // v2, `super::online_prices`).
    let prices = crate::ui::original::ShopPrices::default();
    match super::shop_prices::PriceTables::live(&live.tables) {
        Ok(tables) => app
            .world_mut()
            .insert_resource(super::online_prices::OnlinePrices {
                tables: Arc::new(tables),
                board: prices.clone(),
            }),
        Err(e) => warn!("shop price tables: {e}; the shop shows no prices"),
    }
    play::install_live_client(
        app,
        link,
        LiveClient {
            data: live,
            request: &character,
            start_flags: Some(crate::ui::front_end::screens::difficulty::start_flags(
                join.status,
            )),
            prices,
            speeds,
            hardcore,
            automap_files: None,
            gpu: true,
        },
    )?;
    let w = app.world_mut();
    w.insert_resource(OnlineGame(status));
    w.insert_resource(session.clone());
    w.init_resource::<OnlineEnd>();
    w.insert_resource(play::ExitReport::default());
    Ok(PlayHandles {
        saver: None,
        dev_relaunch: Default::default(),
    })
}

/// The join of a lobby ticket on a worker thread: [`JoinTask::poll`]
/// answers without waiting (contracts 10.1: no blocking call on the game's
/// thread).
pub struct JoinTask(Mutex<mpsc::Receiver<Result<GameLink, OnlineError>>>);

impl JoinTask {
    /// Starts joining the game of `ticket` through `session`.
    pub fn start(session: &OnlineSession, ticket: [u8; 16]) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let shared = session.clone();
        std::thread::Builder::new()
            .name("d2-online-join".into())
            .spawn(move || {
                let _ = tx.send(join_game(&shared, ticket));
            })?;
        Ok(Self(Mutex::new(rx)))
    }

    /// The server's answer once it came (`None`: still waiting).
    pub fn poll(&self) -> Option<Result<GameLink, OnlineError>> {
        let rx = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match rx.try_recv() {
            Ok(a) => Some(a),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(OnlineError::Lost(NetError::Closed))),
        }
    }
}

/// An online game waiting for the server's answer to its join; the game
/// installs once it comes ([`poll_join`]).
#[derive(Resource)]
pub struct PendingJoin {
    task: JoinTask,
    live: Arc<LiveData>,
    session: OnlineSession,
    join: OnlineJoin,
}

/// Starts the online game of `join` without waiting (contracts 10.1: no
/// blocking call on the game's thread): the join runs on a worker thread
/// and [`poll_join`] installs the game once the server answered, one check
/// per frame. A failed join ends the game with the reason in
/// [`OnlineEnd`].
pub fn start_online(
    world: &mut World,
    live: &Arc<LiveData>,
    session: &OnlineSession,
    join: OnlineJoin,
) -> anyhow::Result<()> {
    join_character(&join)?;
    let task = JoinTask::start(session, join.ticket).context("starting the join")?;
    world.init_resource::<OnlineEnd>();
    world.insert_resource(PendingJoin {
        task,
        live: live.clone(),
        session: session.clone(),
        join,
    });
    Ok(())
}

/// Installs a [`PendingJoin`]'s game once the server answered; a refusal
/// or a lost connection goes back to the front end with the reason.
fn poll_join(world: &mut World) {
    let Some(answer) = world
        .get_resource::<PendingJoin>()
        .and_then(|p| p.task.poll())
    else {
        return;
    };
    let Some(p) = world.remove_resource::<PendingJoin>() else {
        return;
    };
    let failed = match answer {
        Ok(game) => match install_joined(world, &p.live, &p.session, &p.join, game) {
            Ok(_) => return,
            Err(e) => format!("{e:#}"),
        },
        Err(e) => {
            // The connection went with it: the screens go back to login.
            if matches!(e, OnlineError::Lost(_) | OnlineError::Connect(_)) {
                drop_session(&p.session);
            }
            e.message()
        }
    };
    warn!("online: the game did not start: {failed}");
    if let Some(end) = world.get_resource::<OnlineEnd>() {
        end.set(failed);
    }
    world.insert_resource(ToMenu);
}

/// Closes the session after its connection failed for good.
fn drop_session(session: &OnlineSession) {
    if let Some(s) = session.0.lock().ok().and_then(|mut s| s.take()) {
        s.close();
    }
}

/// The online game's systems (registered with the live client): the
/// pending join and the connection watch.
pub(crate) fn add_watch(app: &mut App) {
    app.add_systems(
        Update,
        (
            poll_join.run_if(resource_exists::<PendingJoin>),
            watch_connection.run_if(resource_exists::<OnlineGame>),
        )
            .run_if(in_state(Screen::Game)),
    );
}

/// Ends the game when its connection is lost for good: back to the front
/// end, which shows why ([`OnlineEnd`]). The session went with it, so the
/// screens open at the login.
fn watch_connection(world: &mut World) {
    let Some(Connection::Lost(why)) = world.get_resource::<OnlineGame>().map(|g| g.0.connection())
    else {
        return;
    };
    world.remove_resource::<OnlineGame>();
    warn!("online: {why}");
    if let Some(s) = world.get_resource::<OnlineSession>() {
        drop_session(s);
    }
    if let Some(end) = world.get_resource::<OnlineEnd>() {
        end.set(why);
    }
    world.insert_resource(ToMenu);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn list_text() -> String {
        format!(
            "[[server]]\nid = \"eu1\"\nname = \"D2RUST Europe\"\ngame = \"d2rust\"\n\
             address = \"37.27.64.176:6112\"\ncert_sha256 = \"{FP}\"\nversion = \"0.1.2.0\"\n\n\
             [[server]]\nid = \"v6\"\nname = \"Six\"\ngame = \"d2rust\"\n\
             address = \"[2001:db8::1]:6112\"\ncert_sha256 = \"{}\"\nversion = \"0.1.2.0\"\n",
            FP.to_uppercase()
        )
    }

    #[test]
    fn the_launchers_list_is_read_with_pinned_fingerprints() {
        let l = ServerList::parse(&list_text()).unwrap();
        assert_eq!(l.servers.len(), 2);
        let eu = l.pick(Some("eu1")).unwrap();
        assert_eq!(eu.address, "37.27.64.176:6112");
        assert_eq!(
            eu.trust,
            ServerTrust::Pinned(parse_fingerprint(FP).unwrap())
        );
        assert_eq!(eu.spec().address, eu.address);
        assert_eq!(l.pick(None).unwrap().id, "eu1");
        assert_eq!(l.pick(Some("v6")).unwrap().address, "[2001:db8::1]:6112");
        assert!(l.pick(Some("nope")).is_none());
    }

    #[test]
    fn a_bad_entry_fails_the_list_with_a_reason() {
        let bad = list_text().replace(FP, "abc");
        let e = ServerList::parse(&bad).unwrap_err();
        assert!(e.contains("cert_sha256"), "{e}");
        let missing = list_text().replace("id = \"eu1\"\n", "");
        assert!(ServerList::parse(&missing).unwrap_err().contains("`id`"));
    }

    #[test]
    fn a_missing_file_says_to_start_from_the_launcher() {
        let e = ServerList::read(Path::new("no/such/servers.toml")).unwrap_err();
        assert_eq!(e, NO_LIST);
        assert_eq!(ServerList::parse("").unwrap(), ServerList::default());
    }

    /// Without the launcher's file the game still knows the D2RUST server:
    /// its address, its pinned certificate and this build's version.
    #[test]
    fn without_the_launchers_file_the_builtin_server_is_used() {
        let l = ServerList::load_from(Some(Path::new("no/such/servers.toml"))).unwrap();
        assert_eq!(l, ServerList::builtin());
        assert_eq!(ServerList::load_from(None).unwrap(), l);
        let s = l.pick(None).unwrap();
        assert_eq!(
            (s.id.as_str(), s.address.as_str()),
            ("eu1", "37.27.64.176:6112")
        );
        assert_eq!(
            s.trust,
            ServerTrust::Pinned(parse_fingerprint(DEFAULT_SERVER_CERT).unwrap())
        );
        assert_eq!(s.version, crate::launch::VERSION);
        assert_eq!(s.version, client_info().build);
        // The launcher's file, when there is one, replaces it.
        let dir = std::env::temp_dir().join(format!("d2-servers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(SERVERS_FILE);
        std::fs::write(&file, list_text()).unwrap();
        let from_file = ServerList::load_from(Some(&file)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(from_file, ServerList::parse(&list_text()).unwrap());
    }

    #[test]
    fn the_join_character_carries_the_selected_name_and_class() {
        let join = OnlineJoin {
            ticket: [1; 16],
            name: "Mogger".into(),
            class: 1,
            status: 0,
            difficulty: 2,
        };
        let Character::Named(c) = join_character(&join).unwrap() else {
            panic!("expected a named character");
        };
        assert_eq!((c.class, c.name(), c.difficulty), (1, &b"Mogger"[..], 2));
        let long = OnlineJoin {
            name: "x".repeat(16),
            ..join.clone()
        };
        assert!(join_character(&long).is_err());
    }

    #[test]
    fn the_client_says_this_builds_version_and_rate() {
        let me = client_info();
        assert_eq!(me.build, crate::launch::VERSION);
        assert_eq!(me.tick_rate, TickRate::GAME.hz());
        assert_eq!(me.game, "d2rust");
    }
}
