//! The bug report intake service.
//!
//! The launcher sends a player's bug report here; the service checks it,
//! stores it privately and answers with a report id. It is the only place a
//! report leaves the player's PC for, and it treats every byte it receives as
//! untrusted data: it checks and stores, and never runs, opens or serves a
//! report back.
//!
//! Endpoints:
//! - `POST /v1/installs`: issues a per-install token (the launcher asks once,
//!   on first launch). Rate limited per client address.
//! - `POST /v1/reports`: one report as `multipart/form-data`, one part per
//!   bundle file, each part named by its file name (`report.json`,
//!   `screenshot.png`, `log.txt`, `state.txt`). Needs
//!   `Authorization: Bearer <token>`. Answers `201 {"report_id": ...}`.
//!
//! Limits, all answered with a clear error and nothing stored:
//! - the request body and every bundle limit of `d2_report` (format v1);
//! - at most one part per file, no other part names;
//! - no executable content in any part, whatever its name;
//! - reports per token per day, reports per day overall, and new tokens per
//!   address per day.
//!
//! Storage: each accepted bundle goes to `<storage>/reports/<report id>/`
//! through `d2_report::Bundle::write_dir`, so only the fixed file names are
//! ever written. Tokens are kept only as SHA-256 hashes
//! (`<storage>/installs.txt`). Configuration comes from the environment
//! ([`Config::from_env`]); the repository holds no deployment and no secrets.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use d2_report::{Bundle, MANIFEST_NAME, MAX_TOTAL_BYTES};
use serde_json::json;
use sha2::{Digest, Sha256};

/// One day, in seconds: the window every rate limit counts in.
pub const DAY: u64 = 24 * 60 * 60;
/// Room for multipart framing on top of the bundle's own size limit.
const MULTIPART_OVERHEAD: u64 = 64 * 1024;
/// Parts a report upload may have: the manifest and one per file kind.
const MAX_PARTS: usize = 1 + d2_report::FileKind::ALL.len();

/// The service's settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// Where reports and token hashes are stored. Private: nothing here is
    /// ever served.
    pub storage: PathBuf,
    /// Reports one install may send per day.
    pub reports_per_token: usize,
    /// Reports the service accepts per day in all.
    pub reports_per_day: usize,
    /// Tokens one client address may get per day.
    pub installs_per_addr: usize,
}

impl Config {
    /// Reads the settings from the environment:
    /// `REPORT_STORAGE_DIR` (required), `REPORT_PER_INSTALL_PER_DAY`
    /// (default 10), `REPORT_PER_DAY` (default 1000) and
    /// `REPORT_INSTALLS_PER_ADDR_PER_DAY` (default 5).
    pub fn from_env() -> anyhow::Result<Config> {
        let num = |key: &str, default: usize| -> anyhow::Result<usize> {
            match std::env::var(key) {
                Ok(v) => v
                    .parse()
                    .map_err(|_| anyhow::anyhow!("{key} is not a whole number")),
                Err(_) => Ok(default),
            }
        };
        Ok(Config {
            storage: std::env::var_os("REPORT_STORAGE_DIR")
                .map(PathBuf::from)
                .ok_or_else(|| anyhow::anyhow!("REPORT_STORAGE_DIR is not set"))?,
            reports_per_token: num("REPORT_PER_INSTALL_PER_DAY", 10)?,
            reports_per_day: num("REPORT_PER_DAY", 1000)?,
            installs_per_addr: num("REPORT_INSTALLS_PER_ADDR_PER_DAY", 5)?,
        })
    }
}

/// The clock the rate limits read, in seconds (replaceable in tests).
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// The system clock.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    })
}

/// Events in the last day, per key.
struct Window<K>(HashMap<K, VecDeque<u64>>);

impl<K> Default for Window<K> {
    fn default() -> Self {
        Window(HashMap::new())
    }
}

/// Keys a window holds before it drops the ones with no event in the last
/// day, so a flood of new addresses can't grow it without bound.
const WINDOW_PRUNE_AT: usize = 10_000;

impl<K: std::hash::Hash + Eq> Window<K> {
    /// Counts one event for `key` at `now` if fewer than `max` happened in
    /// the last day; false if the limit is reached.
    fn take(&mut self, key: K, now: u64, max: usize) -> bool {
        if self.0.len() >= WINDOW_PRUNE_AT {
            self.0
                .retain(|_, q| q.back().is_some_and(|&t| t + DAY > now));
        }
        let q = self.0.entry(key).or_default();
        while q.front().is_some_and(|&t| t + DAY <= now) {
            q.pop_front();
        }
        if q.len() >= max {
            return false;
        }
        q.push_back(now);
        true
    }
}

struct Inner {
    config: Config,
    clock: Clock,
    tokens: Mutex<HashSet<[u8; 32]>>,
    per_token: Mutex<Window<[u8; 32]>>,
    per_addr: Mutex<Window<std::net::IpAddr>>,
    all: Mutex<Window<()>>,
}

/// The service's shared state.
#[derive(Clone)]
pub struct Service(Arc<Inner>);

impl Service {
    /// Opens the storage (creating it, private, if needed) and loads the
    /// known token hashes.
    pub fn open(config: Config, clock: Clock) -> anyhow::Result<Service> {
        private_dir(&config.storage.join("reports"))?;
        let mut tokens = HashSet::new();
        if let Ok(text) = std::fs::read_to_string(config.storage.join("installs.txt")) {
            for line in text.lines() {
                if let Some(h) = unhex32(line.trim()) {
                    tokens.insert(h);
                }
            }
        }
        Ok(Service(Arc::new(Inner {
            config,
            clock,
            tokens: Mutex::new(tokens),
            per_token: Mutex::default(),
            per_addr: Mutex::default(),
            all: Mutex::default(),
        })))
    }

    /// The HTTP routes, with the body limit.
    pub fn router(self) -> Router {
        Router::new()
            .route("/v1/installs", post(issue_token))
            .route("/v1/reports", post(upload))
            .layer(DefaultBodyLimit::max(
                (MAX_TOTAL_BYTES + MULTIPART_OVERHEAD) as usize,
            ))
            .with_state(self)
    }
}

/// An error answer: status plus a short reason as JSON.
struct Reject(StatusCode, String);

impl IntoResponse for Reject {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn bad(why: impl ToString) -> Reject {
    Reject(StatusCode::BAD_REQUEST, why.to_string())
}

/// A broken or oversized multipart body (413 when past the body limit).
fn multipart_error(e: axum::extract::multipart::MultipartError) -> Reject {
    Reject(e.status(), e.body_text())
}

fn internal(e: impl std::fmt::Display) -> Reject {
    eprintln!("report-service: {e}");
    Reject(StatusCode::INTERNAL_SERVER_ERROR, "storage failed".into())
}

async fn issue_token(State(s): State<Service>, req: Request) -> Result<Response, Reject> {
    let addr = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
        .ok_or_else(|| bad("no client address"))?;
    let now = (s.0.clock)();
    if !lock(&s.0.per_addr).take(addr, now, s.0.config.installs_per_addr) {
        return Err(Reject(
            StatusCode::TOO_MANY_REQUESTS,
            "too many installs from this address today".into(),
        ));
    }
    let mut token = [0u8; 32];
    getrandom::fill(&mut token).map_err(internal)?;
    let hash: [u8; 32] = Sha256::digest(token).into();
    // The file is appended under the token lock, so lines never interleave.
    let mut tokens = lock(&s.0.tokens);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(s.0.config.storage.join("installs.txt"))
        .map_err(internal)?;
    writeln!(f, "{}", hex(&hash)).map_err(internal)?;
    tokens.insert(hash);
    drop(tokens);
    Ok((StatusCode::CREATED, Json(json!({ "token": hex(&token) }))).into_response())
}

async fn upload(
    State(s): State<Service>,
    headers: HeaderMap,
    mut form: Multipart,
) -> Result<Response, Reject> {
    let hash = install(&s, &headers)?;
    // Every attempt counts, valid or not, so a flood of bad uploads is
    // limited too.
    let now = (s.0.clock)();
    let limited = || {
        Reject(
            StatusCode::TOO_MANY_REQUESTS,
            "too many reports today".into(),
        )
    };
    if !lock(&s.0.per_token).take(hash, now, s.0.config.reports_per_token) {
        return Err(limited());
    }
    if !lock(&s.0.all).take((), now, s.0.config.reports_per_day) {
        return Err(limited());
    }

    let mut manifest = None;
    let mut parts = Vec::new();
    let mut count = 0;
    while let Some(field) = form.next_field().await.map_err(multipart_error)? {
        count += 1;
        if count > MAX_PARTS {
            return Err(bad("too many parts"));
        }
        let name = field.name().unwrap_or("").to_owned();
        let bytes = field.bytes().await.map_err(multipart_error)?.to_vec();
        if looks_executable(&bytes) {
            return Err(bad("executable content is not accepted"));
        }
        if name == MANIFEST_NAME {
            if manifest.replace(bytes).is_some() {
                return Err(bad("report.json appears more than once"));
            }
        } else {
            parts.push((name, bytes));
        }
    }
    let manifest = manifest.ok_or_else(|| bad("report.json is missing"))?;
    let bundle = Bundle::from_parts(&manifest, parts).map_err(bad)?;

    let mut id = [0u8; 16];
    getrandom::fill(&mut id).map_err(internal)?;
    let id: String = id.iter().map(|b| format!("{b:02x}")).collect();
    bundle
        .write_dir(&s.0.config.storage.join("reports").join(&id))
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "report_id": id }))).into_response())
}

/// The install a request comes from: the hash of its known bearer token.
fn install(s: &Service, headers: &HeaderMap) -> Result<[u8; 32], Reject> {
    let unauthorised = || Reject(StatusCode::UNAUTHORIZED, "unknown install token".into());
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(unhex32)
        .ok_or_else(unauthorised)?;
    let hash: [u8; 32] = Sha256::digest(token).into();
    if !lock(&s.0.tokens).contains(&hash) {
        return Err(unauthorised());
    }
    Ok(hash)
}

/// Whether `bytes` start like a program, a script or a package that holds
/// programs: Windows, Linux and macOS executables, `#!` scripts, WebAssembly,
/// ZIP-based packages (jar, apk, msix) and Windows installers (MSI). No
/// bundle file legitimately starts with any of these.
pub fn looks_executable(bytes: &[u8]) -> bool {
    const MAGIC: [&[u8]; 10] = [
        b"\0asm",
        b"PK\x03\x04",
        b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1",
        b"MZ",
        b"\x7fELF",
        b"#!",
        b"\xca\xfe\xba\xbe",
        b"\xfe\xed\xfa\xce",
        b"\xfe\xed\xfa\xcf",
        b"\xcf\xfa\xed\xfe",
    ];
    MAGIC.iter().any(|m| bytes.starts_with(m))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Creates `dir` and its parents, readable by the service's user only.
fn private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let b = s.as_bytes();
    if b.len() != 64 || !b.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}
