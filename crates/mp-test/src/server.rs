//! The server under test: a real process, started fresh for each
//! scenario in a folder of its own, stopped, killed and restarted on the
//! scenario's word.
//!
//! **What the tool needs from a server process** (the ready line): once
//! it listens, the server prints one line on stdout
//!
//! ```text
//! ready listen=127.0.0.1:50123 cert=/path/cert.der game=d2rust caps=connect,game build=0.1.0 protocol=1 tick=25
//! ```
//!
//! `listen` is the address it bound (the tool asks for port 0); the rest
//! is optional: `cert` the certificate it presents (DER; bots pin it) or
//! `cert_sha256` its fingerprint, `caps` what it serves (without it the
//! tool assumes everything its own drivers speak), `game`, `build`,
//! `protocol` and `tick` what a client's hello must say (bots say it; the
//! defaults are this build's own values). A server that exits before the ready line has failed to start,
//! unless it says it can't run games yet: then every scenario that needs a
//! running server is pending.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Which server program, and how to configure it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `mp-test-synth`: the synthetic town behind a real QUIC endpoint.
    Synth {
        /// The binary.
        bin: PathBuf,
    },
    /// `d2rust-server`, configured by a config file.
    D2rust {
        /// The binary.
        bin: PathBuf,
        /// The game data folder it reads.
        game_dir: PathBuf,
    },
}

impl Target {
    /// Its name in reports.
    pub fn name(&self) -> &'static str {
        match self {
            Target::Synth { .. } => "synth",
            Target::D2rust { .. } => "d2rust-server",
        }
    }

    /// The binary.
    pub fn bin(&self) -> &Path {
        match self {
            Target::Synth { bin } | Target::D2rust { bin, .. } => bin,
        }
    }
}

/// What the ready line said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    /// Where it listens.
    pub listen: SocketAddr,
    /// The certificate it presents (DER), when it named one.
    pub cert: Option<Vec<u8>>,
    /// The SHA-256 of its certificate, when it said (`cert_sha256=`).
    pub cert_sha256: Option<[u8; 32]>,
    /// What it serves, when it said.
    pub caps: Option<BTreeSet<String>>,
    /// The game id it runs, when it said.
    pub game: Option<String>,
    /// The game build its clients must have, when it said.
    pub build: Option<String>,
    /// The `d2-proto` protocol version, when it said.
    pub protocol: Option<u32>,
    /// Its tick rate, when it said.
    pub tick: Option<u32>,
}

/// A ready line, before its certificate file is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyLine {
    /// Where it listens.
    pub listen: SocketAddr,
    /// The certificate file it named.
    pub cert: Option<PathBuf>,
    /// Its certificate's SHA-256.
    pub cert_sha256: Option<[u8; 32]>,
    /// What it serves.
    pub caps: Option<BTreeSet<String>>,
    /// The game id it runs.
    pub game: Option<String>,
    /// Its game build.
    pub build: Option<String>,
    /// Its protocol version.
    pub protocol: Option<u32>,
    /// Its tick rate.
    pub tick: Option<u32>,
}

/// Parses a ready line (see the module docs): `None` when the line isn't
/// one; `Err` when it is one without a usable listen address.
pub fn parse_ready(line: &str) -> Option<Result<ReadyLine, String>> {
    let rest = line.trim().strip_prefix("ready ")?;
    let mut r = ReadyLine {
        listen: SocketAddr::from(([0, 0, 0, 0], 0)),
        cert: None,
        cert_sha256: None,
        caps: None,
        game: None,
        build: None,
        protocol: None,
        tick: None,
    };
    let mut listen = None;
    for w in rest.split_whitespace() {
        match w.split_once('=') {
            Some(("listen", v)) => listen = v.parse::<SocketAddr>().ok(),
            Some(("cert", v)) => r.cert = Some(PathBuf::from(v)),
            Some(("cert_sha256", v)) => r.cert_sha256 = hex32(v),
            Some(("caps", v)) => {
                r.caps = Some(
                    v.split(',')
                        .filter(|c| !c.is_empty())
                        .map(str::to_string)
                        .collect(),
                )
            }
            Some(("game", v)) => r.game = Some(v.to_string()),
            Some(("build", v)) => r.build = Some(v.to_string()),
            Some(("protocol", v)) => r.protocol = v.parse().ok(),
            Some(("tick", v)) => r.tick = v.parse().ok(),
            _ => {}
        }
    }
    Some(match listen {
        Some(l) => {
            r.listen = l;
            Ok(r)
        }
        None => Err(format!("ready line without a listen address: {line}")),
    })
}

/// 64 hex digits as 32 bytes.
fn hex32(v: &str) -> Option<[u8; 32]> {
    if v.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(v.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Why a server didn't start.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    /// It says it can't run games yet: scenarios that need it are pending.
    #[error("the server can't run games yet: {0}")]
    NotServing(String),
    /// It failed: a broken rule.
    #[error("{0}")]
    Failed(String),
}

/// The words a server uses when running games isn't built yet.
const NOT_BUILT: &str = "not built yet";

/// How long a server may take to print its ready line.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a stopped server may take to exit before it is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a test server may take to answer a dev command.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// One server process and its folder.
#[derive(Debug)]
pub struct ServerProc {
    target: Target,
    dir: PathBuf,
    settings: Vec<(String, String)>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    /// Everything it printed (both streams), for the report.
    output: Arc<Mutex<Vec<String>>>,
    /// What its ready line said, while it runs.
    pub ready: Option<Ready>,
    /// How many times it was started.
    pub starts: u32,
}

impl ServerProc {
    /// A server of `target` with its folder `dir` (created) and the
    /// scenario's `settings` (`players`, `grace` in ms, `build`).
    pub fn new(
        target: Target,
        dir: PathBuf,
        settings: Vec<(String, String)>,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            target,
            dir,
            settings,
            child: None,
            stdin: None,
            output: Arc::default(),
            ready: None,
            starts: 0,
        })
    }

    fn setting(&self, key: &str) -> Option<&str> {
        self.settings
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The command line for this target.
    fn command(&self) -> std::io::Result<Command> {
        let mut c = Command::new(self.target.bin());
        match &self.target {
            Target::Synth { .. } => {
                c.arg("--listen")
                    .arg("127.0.0.1:0")
                    .arg("--data")
                    .arg(&self.dir);
                for (k, v) in &self.settings {
                    c.arg(format!("--{k}")).arg(v);
                }
            }
            Target::D2rust { game_dir, .. } => {
                let cfg = self.dir.join("d2rust-server.cfg");
                let mut text = format!(
                    "listen = 127.0.0.1:0\ngame_data = {}\ncharacters = characters\nlogs = logs\n",
                    game_dir.display()
                );
                if let Some(p) = self.setting("players") {
                    text.push_str(&format!("max_players = {p}\n"));
                }
                if let Some(ms) = self.setting("grace").and_then(|v| v.parse::<u64>().ok()) {
                    text.push_str(&format!("grace_s = {}\n", ms.div_ceil(1000)));
                }
                // Every other scenario setting is a config key of the same
                // name (test-server keys such as `lock_s`).
                for (k, v) in &self.settings {
                    if !matches!(k.as_str(), "players" | "grace" | "build") {
                        text.push_str(&format!("{k} = {v}\n"));
                    }
                }
                std::fs::write(&cfg, text)?;
                c.arg("--config").arg(cfg);
            }
        }
        c.current_dir(&self.dir);
        Ok(c)
    }

    /// Starts it and waits for its ready line.
    pub fn start(&mut self) -> Result<&Ready, StartError> {
        if self.child.is_some() {
            return Err(StartError::Failed("the server is already running".into()));
        }
        let mut cmd = self
            .command()
            .map_err(|e| StartError::Failed(format!("writing the server config: {e}")))?;
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                StartError::Failed(format!("starting {}: {e}", self.target.bin().display()))
            })?;
        self.starts += 1;
        let (tx, rx) = mpsc::channel();
        for (stream, is_out) in [
            (
                child
                    .stdout
                    .take()
                    .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
                true,
            ),
            (
                child
                    .stderr
                    .take()
                    .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
                false,
            ),
        ] {
            let Some(stream) = stream else { continue };
            let (out, tx) = (self.output.clone(), tx.clone());
            let start = self.starts;
            std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if is_out {
                        if let Some(r) = parse_ready(&line) {
                            let _ = tx.send(r);
                        }
                    }
                    out.lock()
                        .expect("server output")
                        .push(format!("[start {start}] {line}"));
                }
            });
        }
        drop(tx);
        self.stdin = child.stdin.take();
        let deadline = Instant::now() + START_TIMEOUT;
        let answer = loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(r) => break Some(r),
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Ok(Some(_)) = child.try_wait() {
                // Exited: give the readers a moment to finish the output.
                break rx.recv_timeout(Duration::from_millis(500)).ok();
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(StartError::Failed(format!(
                    "no ready line within {} s",
                    START_TIMEOUT.as_secs()
                )));
            }
        };
        match answer {
            Some(Ok(line)) => {
                let cert = match line.cert {
                    Some(p) => Some(std::fs::read(&p).map_err(|e| {
                        StartError::Failed(format!("reading the server cert {}: {e}", p.display()))
                    })?),
                    None => None,
                };
                self.child = Some(child);
                self.ready = Some(Ready {
                    listen: line.listen,
                    cert,
                    cert_sha256: line.cert_sha256,
                    caps: line.caps,
                    game: line.game,
                    build: line.build,
                    protocol: line.protocol,
                    tick: line.tick,
                });
                Ok(self.ready.as_ref().expect("just set"))
            }
            Some(Err(e)) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(StartError::Failed(e))
            }
            None => {
                let status = child.wait().ok();
                std::thread::sleep(Duration::from_millis(100));
                let tail = self.output_tail(8);
                if tail.contains(NOT_BUILT) {
                    Err(StartError::NotServing(tail))
                } else {
                    Err(StartError::Failed(format!(
                        "exited before its ready line ({}): {tail}",
                        status.map_or("?".into(), |s| s.to_string())
                    )))
                }
            }
        }
    }

    /// Stops it cleanly: SIGTERM where there is one (the server saves and
    /// exits), and its stdin closes (the stand-in stops at the end of its
    /// input); killed when it doesn't exit in time.
    pub fn stop(&mut self) {
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            if cfg!(unix) {
                let _ = Command::new("kill")
                    .arg("-TERM")
                    .arg(child.id().to_string())
                    .status();
            }
            let deadline = Instant::now() + STOP_TIMEOUT;
            while Instant::now() < deadline {
                if let Ok(Some(_)) = child.try_wait() {
                    self.ready = None;
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        self.ready = None;
    }

    /// Kills it at once, as a crash would.
    pub fn kill(&mut self) {
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.ready = None;
    }

    /// Runs the server program once with its config and `args` (an
    /// offline admin command such as `--set-password`); `Err` with its
    /// output unless it exits 0.
    pub fn admin(&mut self, args: &[String]) -> Result<(), String> {
        let mut c = match &self.target {
            Target::D2rust { .. } => {
                let mut c = self.command().map_err(|e| e.to_string())?;
                c.args(args);
                c
            }
            Target::Synth { .. } => return Err("the stand-in has no admin commands".into()),
        };
        let out = c
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("running {}: {e}", self.target.bin().display()))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for line in text.lines() {
            self.output
                .lock()
                .expect("server output")
                .push(format!("[admin] {line}"));
        }
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "admin {}: {} ({})",
                args.join(" "),
                out.status,
                text.trim()
            ))
        }
    }

    /// Sends one line to its stdin (a dev command or `set-progression` on
    /// a test server) and waits for its `dev ok`/`dev err` answer, which a
    /// test server prints on one stdout line: the text after `dev ok`, or
    /// the `dev err` line.
    pub fn command_line(&mut self, line: &str) -> Result<String, String> {
        let from = self.output.lock().expect("server output").len();
        match &mut self.stdin {
            Some(s) => writeln!(s, "{line}").map_err(|e| e.to_string())?,
            None => return Err("the server isn't running".into()),
        }
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            {
                let out = self.output.lock().expect("server output");
                for l in out.get(from..).unwrap_or_default() {
                    if let Some(i) = l.find("dev ok") {
                        return Ok(l[i + "dev ok".len()..].trim().to_string());
                    }
                    if let Some(i) = l.find("dev err") {
                        return Err(l[i..].to_string());
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(format!("no dev answer to {line:?} in time"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// `Some(status)` when it exited by itself while it should run.
    pub fn exited(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(s)) => {
                self.child = None;
                self.ready = None;
                Some(s.to_string())
            }
            _ => None,
        }
    }

    /// The last `n` lines it printed.
    pub fn output_tail(&self, n: usize) -> String {
        let out = self.output.lock().expect("server output");
        let from = out.len().saturating_sub(n);
        out[from..].join("\n")
    }

    /// Every line it printed that reports a panic.
    pub fn panics(&self) -> Vec<String> {
        self.output
            .lock()
            .expect("server output")
            .iter()
            .filter(|l| l.contains("panicked"))
            .cloned()
            .collect()
    }

    /// Its folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for ServerProc {
    fn drop(&mut self) {
        self.kill();
    }
}
