//! `d2rust-server`: the multiplayer server (contracts section 1).
//!
//! ```text
//! d2rust-server [--config FILE]                 run the server
//! d2rust-server [--config FILE] --check         read the config, open the folders and the log, stop
//! d2rust-server [--config FILE] --probe         load the game data, play one game with a new
//!                                               character of every class, check their saves
//! d2rust-server --version                       print `d2rust-server <game version>`
//! d2rust-server [--config FILE] --new-identity  make the certificate (once; kept across versions)
//! d2rust-server [--config FILE] --fingerprint   print the certificate's SHA-256 for the manifest
//! d2rust-server [--config FILE] --backup [DIR]  back up accounts and characters (default: a dated
//!                                               folder in `backups`), the server stopped
//! d2rust-server [--config FILE] --set-password ACCOUNT    (the new password on standard input)
//! d2rust-server [--config FILE] --disable ACCOUNT | --enable ACCOUNT | --reindex
//! ```
//!
//! The config file defaults to `d2rust-server.cfg` in the current folder.
//! Running, the server loads the game data, opens the accounts, takes
//! `data/server.lock`, listens on its UDP port and prints one line
//! `ready listen=… cert_sha256=… game=… build=… protocol=… tick=… caps=…`
//! (contracts 10.1). It writes `logs/status.json` every 10 s and answers
//! backup requests from `server-ctl`. SIGTERM or Ctrl+C: it stops taking
//! connections, every game saves its players and closes, and it exits 0.
//!
//! With `dev = test` in its config (test servers only), it also reads
//! `dev CHARACTER COMMAND` lines and `set-progression CHARACTER N` on
//! standard input and answers `dev ok TEXT` or `dev err TEXT` on standard
//! output (contracts 10.1). Its ready line lists the `dev` capability.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use d2_net::wire::{ServerInfo, Timing};
use d2_quic::{fingerprint, fingerprint_hex, Identity, QuicServer};
use d2_server::game::{GameData, Layers};
use d2rust_server::accounts::Accounts;
use d2rust_server::config::ServerConfig;
use d2rust_server::games::{Games, GamesConfig};
use d2rust_server::lobby::{self, Lobby};
use d2rust_server::log::ServerLog;
use d2rust_server::net::{Net, NetConfig};
use d2rust_server::session::{LobbyApi, Sessions};
use d2rust_server::status::{self, ServerLock, Status, STATUS_INTERVAL};

/// The game this server runs.
const GAME: &str = "d2rust";

/// The game version, the same four parts as the game's (`build.rs`).
const VERSION: &str = env!("D2RUST_VERSION");

/// How long a stopping server waits for its games to save and close.
const STOP_WAIT: Duration = Duration::from_secs(60);

const USAGE: &str = "usage: d2rust-server [--config FILE] [--check | --probe | --new-identity | --fingerprint | --backup [DIR] | --set-password ACCOUNT | --set-progression CHARACTER N | --disable ACCOUNT | --enable ACCOUNT | --reindex] | --version";

enum Command {
    Run,
    Check,
    Probe,
    NewIdentity,
    Fingerprint,
    Backup(Option<PathBuf>),
    SetPassword(String),
    SetProgression(String, u8),
    Disable(String, bool),
    Reindex,
}

fn main() -> ExitCode {
    let mut config = PathBuf::from("d2rust-server.cfg");
    let mut command = None;
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        let next = match a.as_str() {
            "--version" => {
                println!("d2rust-server {VERSION}");
                return ExitCode::SUCCESS;
            }
            "--config" => match args.next() {
                Some(p) => {
                    config = PathBuf::from(p);
                    continue;
                }
                None => return usage(),
            },
            "--check" => Command::Check,
            "--probe" => Command::Probe,
            "--new-identity" => Command::NewIdentity,
            "--fingerprint" => Command::Fingerprint,
            "--reindex" => Command::Reindex,
            "--backup" => {
                Command::Backup(args.next_if(|n| !n.starts_with("--")).map(PathBuf::from))
            }
            "--set-password" | "--disable" | "--enable" => {
                let Some(name) = args.next() else {
                    return usage();
                };
                match a.as_str() {
                    "--set-password" => Command::SetPassword(name),
                    "--disable" => Command::Disable(name, true),
                    _ => Command::Disable(name, false),
                }
            }
            "--set-progression" => {
                let (Some(name), Some(level)) = (args.next(), args.next()) else {
                    return usage();
                };
                match level.parse::<u8>() {
                    Ok(n) => Command::SetProgression(name, n),
                    Err(_) => return usage(),
                }
            }
            _ => return usage(),
        };
        if command.replace(next).is_some() {
            return usage();
        }
    }
    match run(&config, command.unwrap_or(Command::Run)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("d2rust-server: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn run(path: &Path, command: Command) -> Result<ExitCode, String> {
    let cfg = ServerConfig::load(path).map_err(|e| e.to_string())?;
    match command {
        Command::Run => serve(path, &cfg),
        Command::Check => check(path, &cfg),
        Command::Probe => {
            let log = Arc::new(
                ServerLog::open(&cfg.logs)
                    .map_err(|e| format!("opening the log in {}: {e}", cfg.logs.display()))?,
            );
            let data = game_data(&cfg.game_data)?;
            let dir = std::env::temp_dir().join(format!("d2rust-probe-{}", std::process::id()));
            let games = GamesConfig {
                grace: Duration::ZERO,
                ..GamesConfig::from_server(&cfg)
            };
            let r = d2rust_server::games::probe::probe(data, games, &dir, log);
            let _ = std::fs::remove_dir_all(&dir);
            println!("{}", r?);
            Ok(ExitCode::SUCCESS)
        }
        Command::NewIdentity => {
            let (id, made) = Identity::load_or_create(&cfg.identity).map_err(|e| e.to_string())?;
            println!(
                "{} {}",
                if made { "made" } else { "kept" },
                cfg.identity.display()
            );
            println!("cert_sha256 = {}", fingerprint_hex(&fingerprint(&id.cert)));
            Ok(ExitCode::SUCCESS)
        }
        Command::Fingerprint => {
            let id = Identity::load(&cfg.identity)
                .map_err(|e| format!("no identity in {}: {e}", cfg.identity.display()))?;
            println!("cert_sha256 = {}", fingerprint_hex(&fingerprint(&id.cert)));
            Ok(ExitCode::SUCCESS)
        }
        Command::Backup(dest) => {
            offline(&cfg)?;
            let accounts = Accounts::open(&cfg).map_err(|e| e.to_string())?;
            let dir = match dest {
                Some(d) => {
                    let n = accounts.backup_into(&d)?;
                    println!("backed up {n} characters to {}", d.display());
                    d
                }
                None => status::backup_now(&accounts, &cfg.backups, cfg.backup_keep)?,
            };
            println!("ok {}", dir.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::SetPassword(name) => {
            offline(&cfg)?;
            let mut password = String::new();
            std::io::stdin()
                .read_line(&mut password)
                .map_err(|e| e.to_string())?;
            let password = password.trim_end_matches(['\r', '\n']);
            let accounts = Accounts::open(&cfg).map_err(|e| e.to_string())?;
            accounts.set_password(&name, password)?;
            println!("password of {name} set");
            Ok(ExitCode::SUCCESS)
        }
        Command::Disable(name, disabled) => {
            offline(&cfg)?;
            let accounts = Accounts::open(&cfg).map_err(|e| e.to_string())?;
            accounts.set_disabled(&name, disabled)?;
            println!("{name} {}", if disabled { "disabled" } else { "enabled" });
            Ok(ExitCode::SUCCESS)
        }
        Command::SetProgression(name, progression) => {
            offline(&cfg)?;
            let accounts = Accounts::open(&cfg).map_err(|e| e.to_string())?;
            accounts.set_progression(&name, progression)?;
            println!("{name} progression set to {progression}");
            Ok(ExitCode::SUCCESS)
        }
        Command::Reindex => {
            offline(&cfg)?;
            let accounts = Accounts::open(&cfg).map_err(|e| e.to_string())?;
            let r = accounts.reindex()?;
            println!("{} characters reindexed", r.rows);
            for s in &r.skipped {
                println!("skipped: {s}");
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Admin commands change files the running server holds: refused while it
/// runs.
fn offline(cfg: &ServerConfig) -> Result<(), String> {
    match ServerLock::running(&cfg.accounts.data) {
        Some(pid) => Err(format!(
            "the server runs (process {pid}); stop it first (server-ctl stop)"
        )),
        None => Ok(()),
    }
}

/// `--check`: the config, the folders and the log, without the game data
/// load or the network.
fn check(path: &Path, cfg: &ServerConfig) -> Result<ExitCode, String> {
    if !cfg.game_data.is_dir() {
        return Err(format!(
            "game data folder {} doesn't exist",
            cfg.game_data.display()
        ));
    }
    let log = ServerLog::open(&cfg.logs)
        .map_err(|e| format!("opening the log in {}: {e}", cfg.logs.display()))?;
    let accounts = Accounts::open(cfg).map_err(|e| e.to_string())?;
    let identity = match Identity::load(&cfg.identity) {
        Ok(id) => fingerprint_hex(&fingerprint(&id.cert)),
        Err(_) => "none yet (--new-identity)".into(),
    };
    log.line(&format!(
        "config {}: listen {}, max {} players, grace {} s",
        path.display(),
        cfg.listen,
        cfg.max_players,
        cfg.grace.as_secs(),
    ));
    println!("version:    {VERSION}");
    println!("config:     {}", path.display());
    println!("listen:     {}", cfg.listen);
    println!(
        "players:    {} per game, {} games",
        cfg.max_players, cfg.max_games
    );
    println!("game data:  {}", cfg.game_data.display());
    println!("data:       {}", accounts.config().data.display());
    println!("identity:   {identity}");
    println!("log:        {}", log.path().display());
    Ok(ExitCode::SUCCESS)
}

/// The game data: a converted native folder (it has `base/`) or a game
/// install's archives.
fn game_data(dir: &Path) -> Result<GameData, String> {
    let data = if dir.join("base").is_dir() {
        GameData::select_native_with(dir, &Layers::base())
    } else {
        GameData::select(Some(dir))
    };
    data.map_err(|e| format!("game data {}: {e}", dir.display()))
}

/// Runs the server until SIGTERM or Ctrl+C.
fn serve(path: &Path, cfg: &ServerConfig) -> Result<ExitCode, String> {
    let started = Instant::now();
    let log = Arc::new(
        ServerLog::open(&cfg.logs)
            .map_err(|e| format!("opening the log in {}: {e}", cfg.logs.display()))?,
    );
    let _lock = ServerLock::take(&cfg.accounts.data)?;
    log.line(&format!("starting {VERSION} with {}", path.display()));
    if cfg.dev_test {
        log.line("WARNING: dev = test: dev commands on standard input (test servers only)");
    }
    let (identity, made) = Identity::load_or_create(&cfg.identity).map_err(|e| e.to_string())?;
    let cert = fingerprint_hex(&fingerprint(&identity.cert));
    if made {
        log.line(&format!("made the identity in {}", cfg.identity.display()));
    }
    let t = Instant::now();
    let data = game_data(&cfg.game_data)?;
    log.line(&format!(
        "game data loaded in {} ms",
        t.elapsed().as_millis()
    ));
    let accounts = Accounts::open(cfg)
        .map_err(|e| e.to_string())?
        .with_log(log.clone());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cfg.net_threads)
        .thread_name("d2rust-net")
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let games = Games::new(GamesConfig::from_server(cfg), data, log.clone());
    let sessions = Sessions::default();
    let lobby = Arc::new(Lobby::new(
        lobby::limits(cfg),
        games.clone(),
        sessions.clone(),
        accounts.clone(),
    ));
    let caps = caps(lobby.as_ref(), cfg.dev_test);
    let server = ServerInfo {
        protocol: d2_proto::PROTOCOL_VERSION,
        game: GAME.into(),
        build: VERSION.into(),
        tick_rate: d2_sim::tick::TickRate::GAME.hz(),
    };
    let net = Arc::new(Net::new(
        NetConfig {
            server: server.clone(),
            server_name: cfg.server_name.clone(),
            max_players: cfg.max_players as u32,
            max_sessions: cfg.max_sessions,
            grace: cfg.grace,
        },
        accounts.clone(),
        lobby,
        games.clone(),
        sessions,
        log.clone(),
    ));
    let (events_tx, events_rx) = tokio::sync::mpsc::channel(1024);
    games.set_events(events_tx);
    // `dev = test`: the test tool's commands on standard input (contracts
    // 10.1). A server without it refuses every line with `dev err disabled`.
    let dev_games = games.clone();
    let dev_accounts = accounts.clone();
    let dev_enabled = cfg.dev_test;
    std::thread::Builder::new()
        .name("dev-stdin".into())
        .spawn(move || d2rust_server::devcmd::serve(dev_games, dev_accounts, dev_enabled))
        .map_err(|e| format!("starting the dev command reader: {e}"))?;
    let quic = rt.block_on(async {
        QuicServer::bind(
            cfg.listen,
            identity,
            Timing::default(),
            tokio::runtime::Handle::current(),
        )
    });
    let quic = Arc::new(quic.map_err(|e| format!("listening on {}: {e}", cfg.listen))?);
    let addr = quic.local_addr().map_err(|e| e.to_string())?;
    log.line(&format!("listening on {addr}, cert_sha256 {cert}"));
    println!(
        "ready listen={addr} cert_sha256={cert} game={GAME} build={VERSION} protocol={} tick={} caps={}",
        server.protocol,
        server.tick_rate,
        caps.join(",")
    );
    let games_events = games.clone();
    let stop = rt.block_on(async {
        tokio::spawn(net.clone().run(quic.clone()));
        tokio::spawn(net.clone().run_events(events_rx));
        let tick_net = net.clone();
        let tick = {
            let (cfg, games, accounts, log) =
                (cfg.clone(), games.clone(), accounts.clone(), log.clone());
            tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_secs(1));
                let mut n: u64 = 0;
                loop {
                    every.tick().await;
                    tick_net.expire_held().await;
                    games.reap();
                    n += 1;
                    if n.is_multiple_of(STATUS_INTERVAL.as_secs()) {
                        let s = Status {
                            version: VERSION.into(),
                            uptime_s: started.elapsed().as_secs(),
                            sessions: tick_net.session_count(),
                            games: games.list().len(),
                            players: games.players(),
                            tick_overruns: games.tick_overruns(),
                        };
                        if let Err(e) = s.write(&cfg.logs) {
                            log.line(&format!("status.json: {e}"));
                        }
                        let (folder, accounts) = (cfg.folder.clone(), accounts.clone());
                        let (backups, keep) = (cfg.backups.clone(), cfg.backup_keep);
                        let _ = tokio::task::spawn_blocking(move || {
                            status::answer_backup_request(&folder, &accounts, &backups, keep)
                        })
                        .await;
                    }
                }
            })
        };
        let why = wait_for_stop().await;
        tick.abort();
        why
    });
    log.line(&format!("stopping ({stop}): saving every game"));
    quic.close();
    games_events.shutdown_all(STOP_WAIT);
    rt.shutdown_timeout(Duration::from_secs(2));
    log.line("stopped");
    Ok(ExitCode::SUCCESS)
}

/// What this build serves (the ready line's `caps`).
fn caps<L: LobbyApi>(lobby: &L, dev_test: bool) -> Vec<&'static str> {
    caps_with(lobby.caps(), dev_test)
}

/// [`caps`] from the lobby's own capabilities.
fn caps_with(lobby_caps: &[&'static str], dev_test: bool) -> Vec<&'static str> {
    let mut caps = vec!["connect", "accounts", "characters", "admin"];
    caps.extend_from_slice(lobby_caps);
    // Games are entered through the lobby's tickets. A game runs the
    // game's own rules: walking, combat, items, waypoints and acts. Trading
    // between players and parties are not served yet.
    if lobby_caps.contains(&"lobby") {
        caps.extend_from_slice(&[
            "game",
            "persist",
            "walk",
            "combat",
            "items",
            "waypoints",
            "acts",
        ]);
    }
    // A test server takes dev commands on standard input (contracts 10.1).
    if dev_test {
        caps.push("dev");
    }
    caps
}

/// Waits for SIGTERM or Ctrl+C; says which.
async fn wait_for_stop() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return "Ctrl+C";
            }
        };
        tokio::select! {
            _ = term.recv() => "SIGTERM",
            _ = tokio::signal::ctrl_c() => "Ctrl+C",
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "Ctrl+C"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_is_listed_only_with_dev_test() {
        let listed = caps(&d2rust_server::session::NoLobby, false);
        assert!(!listed.contains(&"dev"), "{listed:?}");
        let listed = caps(&d2rust_server::session::NoLobby, true);
        assert!(listed.contains(&"dev"), "{listed:?}");
        for cap in ["connect", "accounts", "characters", "admin"] {
            assert!(listed.contains(&cap), "{cap}: {listed:?}");
        }
    }

    /// The game's play is served with the lobby's games, and only then;
    /// trading between players and parties are not.
    #[test]
    fn play_is_listed_with_the_lobbys_games() {
        const PLAY: [&str; 7] = [
            "game",
            "persist",
            "walk",
            "combat",
            "items",
            "waypoints",
            "acts",
        ];
        let listed = caps_with(&["lobby", "chat"], false);
        for cap in PLAY {
            assert!(listed.contains(&cap), "{cap}: {listed:?}");
        }
        for cap in ["trade", "party"] {
            assert!(!listed.contains(&cap), "{cap}: {listed:?}");
        }
        let listed = caps(&d2rust_server::session::NoLobby, false);
        for cap in PLAY {
            assert!(!listed.contains(&cap), "{cap}: {listed:?}");
        }
    }
}
