//! Accounts: register, log in, and the online characters an account owns
//! (contracts section 4).
//!
//! - **Storage:** one SQLite database, `<data>/accounts.sqlite`, with the
//!   accounts and a summary row per character ([`db`]), and one file per
//!   character, `<data>/characters/<account id>/<name>.d2rc`, in the
//!   character envelope of [`crate::characters`]. The file is the truth;
//!   the row is what character select and the lobby read, written after
//!   every save ([`Accounts::saved`]) and rebuilt by [`Accounts::reindex`].
//! - **Passwords** are hashed with argon2id ([`hash`]); at most two hashes
//!   run at a time, off the network threads. Every failed login is
//!   answered after the same fixed delay, so the answer's timing says
//!   nothing about why it failed.
//! - **Limits** on registering and logging in are in [`limits`].
//! - **Characters:** names are unique on the server in any case. A new
//!   character is only a row until its first game makes and saves its
//!   file. A delete marks the row and moves the file to
//!   `<data>/deleted/<account id>/`; the name stays taken for
//!   [`NAME_HOLD`], then it is free again.
//! - Sessions (who is logged in on which connection) live with the
//!   connections, not here: a server restart ends them all.
//!
//! Logs never carry passwords or hashes.

pub mod db;
pub mod hash;
pub mod limits;
pub mod rules;
pub mod summary;

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use d2_formats::d2s::status;

use d2_net::msg::account::{CharSummary, CLASSES, MAX_CHARACTERS_LIMIT};
use d2_net::msg::ErrorCode;

use crate::characters::{self, CharacterStore, EXTENSION};
use crate::config::ServerConfig;
pub use crate::denied::Denied;
use crate::log::ServerLog;
use db::Db;
pub use hash::HashCost;
use limits::Limits;

/// How long a deleted character's name stays taken.
pub const NAME_HOLD: Duration = Duration::from_secs(7 * 24 * 3600);
/// Characters per account unless the config says otherwise.
pub const DEFAULT_MAX_CHARACTERS: u8 = 16;
/// How long every failed login waits before its answer.
pub const FAIL_DELAY: Duration = Duration::from_millis(500);
/// The failed-login delay in the test limits mode (`limits = test`).
pub const TEST_FAIL_DELAY: Duration = Duration::from_millis(10);
/// Password hashes that may run at once.
const HASHES_AT_ONCE: usize = 2;
/// The database file in the data folder.
pub const DATABASE: &str = "accounts.sqlite";

/// An account's row id in the accounts database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(pub i64);

impl std::fmt::Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "account {}", self.0)
    }
}

/// A logged-in account: its id and its name as stored, with its case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
}

/// A character the account created that has never been played: the store
/// holds only its summary until its first game makes and saves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewChar {
    pub name: String,
    /// Class id 0-6.
    pub class: u8,
    pub hardcore: bool,
    pub expansion: bool,
    /// The progression bits (0-31, acts completed across the difficulties)
    /// the character starts with: what `--set-progression` stored before
    /// its first game. The game writes them into the new record, so its
    /// first save keeps them.
    pub progression: u8,
}

/// What a game needs to seat a character: its summary, its file, and how
/// to make it when it has never been played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharSeat {
    pub summary: CharSummary,
    pub file: PathBuf,
    /// `Some` while the character has no file yet.
    pub new: Option<NewChar>,
}

/// The accounts part of the server config.
///
/// ```text
/// data           = data        # the database and the character files
/// argon2         = normal      # or `test`, for test servers only
/// limits         = normal      # or `test`, for test servers only
/// max_characters = 16          # per account, 1 to 32
/// registration   = open        # or `closed`: no new accounts
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountsConfig {
    pub data: PathBuf,
    pub cost: HashCost,
    /// `test` divides every rate-limit window and lock by 60 and makes the
    /// failed-login delay [`TEST_FAIL_DELAY`].
    pub limits: LimitMode,
    pub max_characters: u8,
    pub registration_open: bool,
    /// The wait before every failed login's answer ([`FAIL_DELAY`]; not a
    /// file key). Ignored in the test limits mode, which uses
    /// [`TEST_FAIL_DELAY`].
    pub fail_delay: Duration,
}

/// The rate limits' mode (`limits`, contracts 5 and 10.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitMode {
    /// The normal windows.
    Normal,
    /// The test windows: every window and lock divided by 60, and the
    /// failed-login delay 10 ms.
    Test,
}

impl AccountsConfig {
    /// The config keys this part reads.
    pub const KEYS: &'static [&'static str] =
        &["data", "argon2", "limits", "max_characters", "registration"];

    /// The defaults for a server folder at `base`.
    pub fn new(base: &Path) -> Self {
        Self {
            data: base.join("data"),
            cost: HashCost::Normal,
            limits: LimitMode::Normal,
            max_characters: DEFAULT_MAX_CHARACTERS,
            registration_open: true,
            fail_delay: FAIL_DELAY,
        }
    }

    /// Sets one of [`Self::KEYS`] from its text; a relative path is taken
    /// from `base`.
    pub fn set(&mut self, key: &str, value: &str, base: &Path) -> Result<(), String> {
        match key {
            "data" => self.data = base.join(value),
            "argon2" => {
                self.cost = match value {
                    "normal" => HashCost::Normal,
                    "test" => HashCost::Test,
                    _ => return Err("must be `normal` or `test`".into()),
                }
            }
            "limits" => {
                self.limits = match value {
                    "normal" => LimitMode::Normal,
                    "test" => LimitMode::Test,
                    _ => return Err("must be `normal` or `test`".into()),
                }
            }
            "max_characters" => {
                let v = value.parse::<u8>().map_err(|e| e.to_string())?;
                if v == 0 || usize::from(v) > MAX_CHARACTERS_LIMIT {
                    return Err(format!("must be 1 to {MAX_CHARACTERS_LIMIT}"));
                }
                self.max_characters = v;
            }
            "registration" => {
                self.registration_open = match value {
                    "open" => true,
                    "closed" => false,
                    _ => return Err("must be `open` or `closed`".into()),
                }
            }
            _ => return Err(format!("`{key}` is not an accounts key")),
        }
        Ok(())
    }

    pub fn database(&self) -> PathBuf {
        self.data.join(DATABASE)
    }

    pub fn characters_dir(&self) -> PathBuf {
        self.data.join("characters")
    }

    pub fn deleted_dir(&self) -> PathBuf {
        self.data.join("deleted")
    }
}

/// Opening the accounts failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    #[error(transparent)]
    Db(#[from] db::DbError),
    #[error("{0}")]
    Io(String),
}

/// What a reindex did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reindexed {
    /// Rows written from character files.
    pub rows: usize,
    /// Files left out, each with why.
    pub skipped: Vec<String>,
}

/// The accounts service. Cheap to clone: clones share one database.
#[derive(Clone)]
pub struct Accounts {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: AccountsConfig,
    db: Mutex<Db>,
    limits: Mutex<Limits>,
    hashing: tokio::sync::Semaphore,
    /// A hash checked for names with no account, so an unknown name costs
    /// the same as a wrong password.
    dummy: String,
    log: Option<Arc<ServerLog>>,
}

impl std::fmt::Debug for Accounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Accounts")
            .field("data", &self.inner.cfg.data)
            .finish_non_exhaustive()
    }
}

/// Seconds since the Unix epoch.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn io(e: std::io::Error) -> AccountError {
    AccountError::Io(e.to_string())
}

impl Accounts {
    /// Opens the accounts of the server config.
    pub fn open(cfg: &ServerConfig) -> Result<Self, AccountError> {
        Self::open_with(cfg.accounts.clone())
    }

    /// Opens the accounts in `cfg.data`, creating the folder and the
    /// database if needed.
    pub fn open_with(cfg: AccountsConfig) -> Result<Self, AccountError> {
        std::fs::create_dir_all(cfg.characters_dir()).map_err(io)?;
        let db = Db::open(&cfg.database())?;
        let dummy = hash::hash(cfg.cost, &format!("{:x}", unix_now()));
        let limits = match cfg.limits {
            LimitMode::Normal => Limits::new(),
            LimitMode::Test => Limits::test(),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                cfg,
                db: Mutex::new(db),
                limits: Mutex::new(limits),
                hashing: tokio::sync::Semaphore::new(HASHES_AT_ONCE),
                dummy,
                log: None,
            }),
        })
    }

    /// Writes failures and account events to `log`, and warns about the
    /// test-only settings in it. Call before cloning: it does nothing once
    /// `self` has been cloned.
    pub fn with_log(mut self, log: Arc<ServerLog>) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            if inner.cfg.cost == HashCost::Test {
                log.line(
                    "accounts: WARNING: argon2 = test: weak password hashing (test servers only)",
                );
            }
            if inner.cfg.limits == LimitMode::Test {
                log.line(
                    "accounts: WARNING: limits = test: shortened rate limits and 10 ms failed-login delay (test servers only)",
                );
            }
            inner.log = Some(log);
        }
        self
    }

    /// The wait before a failed login's answer: the configured one, or
    /// [`TEST_FAIL_DELAY`] in the test limits mode.
    fn fail_delay(&self) -> Duration {
        match self.inner.cfg.limits {
            LimitMode::Normal => self.inner.cfg.fail_delay,
            LimitMode::Test => TEST_FAIL_DELAY,
        }
    }

    pub fn config(&self) -> &AccountsConfig {
        &self.inner.cfg
    }

    fn log(&self, line: &str) {
        if let Some(log) = &self.inner.log {
            log.line(&format!("accounts: {line}"));
        }
    }

    /// Runs `f` on the database off the network threads. A database
    /// failure is logged and answered as [`ErrorCode::OTHER`].
    async fn db<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Db) -> Result<T, db::DbError> + Send + 'static,
    ) -> Result<T, Denied> {
        let inner = self.inner.clone();
        let r = tokio::task::spawn_blocking(move || {
            let mut db = inner.db.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut db)
        })
        .await
        .unwrap_or_else(|e| Err(db::DbError(format!("task failed: {e}"))));
        r.map_err(|e| {
            self.log(&e.to_string());
            Denied::because(
                ErrorCode::OTHER,
                "The account store failed. Try again later.",
            )
        })
    }

    /// Checks `password` against `phc` (or the dummy hash), at most
    /// [`HASHES_AT_ONCE`] at a time, off the network threads.
    async fn verify(&self, phc: Option<String>, password: &str) -> bool {
        let _permit = self.inner.hashing.acquire().await;
        let inner = self.inner.clone();
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || {
            let phc = phc.as_deref().unwrap_or(&inner.dummy);
            hash::verify(phc, &password)
        })
        .await
        .unwrap_or(false)
    }

    async fn hash(&self, password: &str) -> String {
        let _permit = self.inner.hashing.acquire().await;
        let cost = self.inner.cfg.cost;
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || hash::hash(cost, &password))
            .await
            .expect("hashing never panics")
    }

    fn limits(&self) -> std::sync::MutexGuard<'_, Limits> {
        self.inner.limits.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Makes an account and logs it in.
    pub async fn register(
        &self,
        name: &str,
        password: &str,
        ip: IpAddr,
    ) -> Result<Account, Denied> {
        if !self.inner.cfg.registration_open {
            return Err(ErrorCode::REGISTRATION_CLOSED.into());
        }
        rules::check_account_name(name)
            .map_err(|t| Denied::because(ErrorCode::BAD_ACCOUNT_NAME, t))?;
        rules::check_password(password).map_err(|t| Denied::because(ErrorCode::BAD_PASSWORD, t))?;
        let now = Instant::now();
        // Check and reserve the slot in one step: registrations that run
        // at the same time must not all pass a check made before any of
        // them is counted.
        self.limits()
            .reserve_register(ip, now)
            .map_err(|w| Denied::wait(ErrorCode::RATE_LIMITED, w))?;
        let key = rules::name_key(name);
        // A taken name answers before any hashing. Probing names costs
        // login attempts.
        let taken = {
            let key = key.clone();
            self.db(move |db| db.account_by_key(&key)).await?.is_some()
        };
        if taken {
            self.limits().release_register(ip, now);
            let _ = self.limits().take_login(ip, now);
            return Err(ErrorCode::ACCOUNT_NAME_TAKEN.into());
        }
        let pw_hash = self.hash(password).await;
        let (stored, at) = (name.to_owned(), unix_now());
        let made = self
            .db(move |db| {
                let id = db.insert_account(&stored, &key, &pw_hash, at)?;
                if let Some(id) = id {
                    db.set_last_login(id, at)?;
                }
                Ok(id)
            })
            .await;
        let id = match made {
            Ok(Some(id)) => id,
            Ok(None) => {
                self.limits().release_register(ip, now);
                return Err(Denied::new(ErrorCode::ACCOUNT_NAME_TAKEN));
            }
            Err(d) => {
                self.limits().release_register(ip, now);
                return Err(d);
            }
        };
        self.log(&format!("registered `{name}` ({})", AccountId(id)));
        Ok(Account {
            id: AccountId(id),
            name: name.to_owned(),
        })
    }

    /// Logs in. Every failure is answered the configured failed-login
    /// delay after the call began.
    pub async fn login(&self, name: &str, password: &str, ip: IpAddr) -> Result<Account, Denied> {
        let start = tokio::time::Instant::now();
        let r = self.try_login(name, password, ip).await;
        if r.is_err() {
            tokio::time::sleep_until(start + self.fail_delay()).await;
        }
        r
    }

    async fn try_login(&self, name: &str, password: &str, ip: IpAddr) -> Result<Account, Denied> {
        let key = rules::name_key(name);
        {
            let now = Instant::now();
            let mut limits = self.limits();
            limits
                .take_login(ip, now)
                .map_err(|w| Denied::wait(ErrorCode::RATE_LIMITED, w))?;
            if let Some(w) = limits.locked(&key, now) {
                return Err(Denied::wait(ErrorCode::ACCOUNT_LOCKED, w));
            }
        }
        let row = if rules::check_account_name(name).is_ok() {
            let key = key.clone();
            self.db(move |db| db.account_by_key(&key)).await?
        } else {
            None
        };
        let good = rules::check_password(password).is_ok()
            && self
                .verify(row.as_ref().map(|r| r.pw_hash.clone()), password)
                .await
            && row.is_some();
        let Some(row) = row.filter(|_| good) else {
            self.limits().note_failure(&key, Instant::now());
            return Err(ErrorCode::WRONG_LOGIN.into());
        };
        // The name may have locked while this password was being checked
        // (attempts that passed the first check at the same time). The
        // lock wins over a good password, so a queued guess can't slip in.
        if let Some(w) = self.limits().locked(&key, Instant::now()) {
            return Err(Denied::wait(ErrorCode::ACCOUNT_LOCKED, w));
        }
        if row.disabled {
            return Err(ErrorCode::ACCOUNT_DISABLED.into());
        }
        self.limits().clear_failures(&key);
        let id = row.id;
        self.db(move |db| db.set_last_login(id, unix_now())).await?;
        Ok(Account {
            id: AccountId(row.id),
            name: row.name,
        })
    }

    /// The account's characters, oldest first. A database failure is
    /// logged and gives an empty list.
    pub async fn characters(&self, a: AccountId) -> Vec<CharSummary> {
        self.db(move |db| db.characters(a.0))
            .await
            .unwrap_or_default()
    }

    /// Creates a character on the account. It is a summary row until its
    /// first game saves it.
    pub async fn create_character(
        &self,
        a: AccountId,
        new: NewChar,
    ) -> Result<CharSummary, Denied> {
        if let Err(e) = characters::check_name(&new.name) {
            let text = match e {
                characters::CharacterError::BadName { reason, .. } => reason,
                _ => "That character name isn't allowed.",
            };
            return Err(Denied::because(ErrorCode::BAD_CHAR_NAME, text));
        }
        if new.class >= CLASSES {
            return Err(Denied::because(
                ErrorCode::OTHER,
                "That class doesn't exist.",
            ));
        }
        // Druid (5) and Assassin (6) are expansion classes.
        if new.class >= 5 && !new.expansion {
            return Err(ErrorCode::EXPANSION_CLASS.into());
        }
        if new.progression > 0x1F {
            return Err(Denied::because(
                ErrorCode::OTHER,
                "That progression doesn't exist.",
            ));
        }
        let s = summary::of_new(&new);
        let max = usize::from(self.inner.cfg.max_characters);
        let row = s.clone();
        let hold = i64::try_from(NAME_HOLD.as_secs()).unwrap_or(i64::MAX);
        let made = self
            .db(move |db| {
                if db.count_characters(a.0)? >= max {
                    return Ok(Err(ErrorCode::CHAR_LIMIT));
                }
                let now = unix_now();
                Ok(if db.insert_character(a.0, &row, now, now - hold)? {
                    Ok(())
                } else {
                    Err(ErrorCode::CHAR_NAME_TAKEN)
                })
            })
            .await?;
        made.map_err(Denied::new)?;
        self.log(&format!("{a} created character `{}`", s.name));
        Ok(s)
    }

    /// Deletes a character of the account; `password` is the account's,
    /// to confirm. A wrong password counts as a failed login and is
    /// answered after the same delay. The caller first refuses a
    /// character that is in a game.
    pub async fn delete_character(
        &self,
        a: AccountId,
        name: &str,
        password: &str,
    ) -> Result<(), Denied> {
        let start = tokio::time::Instant::now();
        let r = self.try_delete(a, name, password).await;
        if matches!(&r, Err(d) if d.code == ErrorCode::WRONG_LOGIN) {
            tokio::time::sleep_until(start + self.fail_delay()).await;
        }
        r
    }

    async fn try_delete(&self, a: AccountId, name: &str, password: &str) -> Result<(), Denied> {
        let account = self
            .db(move |db| db.account_by_id(a.0))
            .await?
            .ok_or(Denied::new(ErrorCode::NOT_LOGGED_IN))?;
        let key = rules::name_key(&account.name);
        if let Some(w) = self.limits().locked(&key, Instant::now()) {
            return Err(Denied::wait(ErrorCode::ACCOUNT_LOCKED, w));
        }
        if !self.verify(Some(account.pw_hash), password).await {
            self.limits().note_failure(&key, Instant::now());
            return Err(ErrorCode::WRONG_LOGIN.into());
        }
        // The name may have locked while this password was being checked.
        if let Some(w) = self.limits().locked(&key, Instant::now()) {
            return Err(Denied::wait(ErrorCode::ACCOUNT_LOCKED, w));
        }
        let char_key = rules::name_key(name);
        let row = {
            let char_key = char_key.clone();
            self.db(move |db| db.character(&char_key)).await?
        }
        .filter(|r| r.account_id == a.0 && r.deleted.is_none())
        .ok_or(Denied::new(ErrorCode::UNKNOWN_CHAR))?;
        let now = unix_now();
        let deleted = self
            .db(move |db| db.mark_deleted(a.0, &char_key, now))
            .await?;
        if !deleted {
            return Err(ErrorCode::UNKNOWN_CHAR.into());
        }
        let stored = row.summary.name;
        let file = self.file_of(a, &stored);
        if file.exists() {
            let dir = self.inner.cfg.deleted_dir().join(a.0.to_string());
            let moved = std::fs::create_dir_all(&dir).and_then(|()| {
                std::fs::rename(&file, dir.join(format!("{stored}-{now}.{EXTENSION}")))
            });
            if let Err(e) = moved {
                // The row is deleted; the file stays where the reindex
                // would bring it back, so say so.
                self.log(&format!(
                    "{a}: moving deleted `{stored}`'s file failed: {e}"
                ));
            }
        }
        self.log(&format!("{a} deleted character `{stored}`"));
        Ok(())
    }

    /// One of the account's live characters.
    pub async fn character(&self, a: AccountId, name: &str) -> Result<CharSummary, Denied> {
        let key = rules::name_key(name);
        self.db(move |db| db.character(&key))
            .await?
            .filter(|r| r.account_id == a.0 && r.deleted.is_none())
            .map(|r| r.summary)
            .ok_or(Denied::new(ErrorCode::UNKNOWN_CHAR))
    }

    /// What a game needs to seat one of the account's characters.
    pub async fn seat(&self, a: AccountId, name: &str) -> Result<CharSeat, Denied> {
        let summary = self.character(a, name).await?;
        let file = self.file_of(a, &summary.name);
        let new = (!file.exists()).then(|| NewChar {
            name: summary.name.clone(),
            class: summary.class,
            hardcore: summary.status & d2_formats::d2s::status::HARDCORE != 0,
            expansion: summary.status & d2_formats::d2s::status::EXPANSION != 0,
            progression: status::progression(summary.status),
        });
        Ok(CharSeat { summary, file, new })
    }

    fn file_of(&self, a: AccountId, stored: &str) -> PathBuf {
        self.inner
            .cfg
            .characters_dir()
            .join(a.0.to_string())
            .join(format!("{stored}.{EXTENSION}"))
    }

    /// Where the game loads and saves this character's record. `name`
    /// matches the stored name in any case; the path uses the stored one.
    /// Only a live character of this account has a file: an unknown name
    /// (or one of another account's, or a deleted one) is `UnknownChar`,
    /// never a path built from unchecked text.
    pub fn character_file(&self, a: AccountId, name: &str) -> Result<PathBuf, Denied> {
        let key = rules::name_key(name);
        let row = {
            let db = self.inner.db.lock().unwrap_or_else(|p| p.into_inner());
            db.character(&key).ok().flatten()
        }
        .filter(|r| r.account_id == a.0 && r.deleted.is_none())
        .ok_or(Denied::new(ErrorCode::UNKNOWN_CHAR))?;
        Ok(self.file_of(a, &row.summary.name))
    }

    /// The account's character folder, created if needed (a game calls it
    /// before its first save of the account's character).
    pub fn character_store(
        &self,
        a: AccountId,
    ) -> Result<CharacterStore, characters::CharacterError> {
        CharacterStore::open(self.inner.cfg.characters_dir().join(a.0.to_string()))
    }

    /// Called by a game after it saved a character: its row takes the
    /// saved summary. A progression set before the character's first game
    /// ([`Accounts::set_progression`]) is seeded into the record here: the
    /// game makes a new record without one.
    pub async fn saved(&self, a: AccountId, mut summary: CharSummary) {
        let name = summary.name.clone();
        let seeded = {
            let db = self.inner.db.lock().unwrap_or_else(|p| p.into_inner());
            db.character(&rules::name_key(&summary.name))
                .ok()
                .flatten()
                .filter(|r| {
                    r.account_id == a.0 && r.deleted.is_none() && r.summary.last_played == 0
                })
                .map(|r| (status::progression(r.summary.status), r.summary.name))
        };
        if let Some((p, stored)) = seeded {
            if p > 0 && status::progression(summary.status) < p {
                let st = (summary.status & !(0x1F << 8)) | (u16::from(p) << 8);
                let file = self.file_of(a, &stored);
                if file.exists() {
                    let write = (|| -> Result<(), String> {
                        let store = CharacterStore::open(file.parent().unwrap_or(Path::new(".")))
                            .map_err(|e| e.to_string())?;
                        let mut record = store.load(&stored).map_err(|e| e.to_string())?;
                        if !summary::set_status(&mut record, st) {
                            return Err("the record has no status word".into());
                        }
                        store.save(&stored, &record).map_err(|e| e.to_string())
                    })();
                    if let Err(e) = write {
                        self.log(&format!("{a}: seeding `{name}`'s progression failed: {e}"));
                    }
                }
                summary.status = st;
                summary.difficulties = status::difficulties(st);
            }
        }
        match self.db(move |db| db.save_summary(a.0, &summary)).await {
            Ok(true) => {}
            Ok(false) => self.log(&format!("{a}: saved `{name}` has no live row")),
            Err(_) => {}
        }
    }

    // ------------------------------------------------- admin (blocking)

    /// Sets an account's password (the admin's reset; there is no email).
    pub fn set_password(&self, name: &str, password: &str) -> Result<(), String> {
        rules::check_password(password)?;
        let h = hash::hash(self.inner.cfg.cost, password);
        let db = self.inner.db.lock().unwrap_or_else(|p| p.into_inner());
        if db
            .set_password(&rules::name_key(name), &h)
            .map_err(|e| e.to_string())?
        {
            self.log(&format!("admin set the password of `{name}`"));
            Ok(())
        } else {
            Err(format!("no account named `{name}`"))
        }
    }

    /// Disables (or enables again) an account: a disabled account can't
    /// log in.
    pub fn set_disabled(&self, name: &str, disabled: bool) -> Result<(), String> {
        let db = self.inner.db.lock().unwrap_or_else(|p| p.into_inner());
        if db
            .set_disabled(&rules::name_key(name), disabled)
            .map_err(|e| e.to_string())?
        {
            let what = if disabled { "disabled" } else { "enabled" };
            self.log(&format!("admin {what} `{name}`"));
            Ok(())
        } else {
            Err(format!("no account named `{name}`"))
        }
    }

    /// Sets the progression bits (0-31) of the online character `name`
    /// (any case, any account) in its summary row and, when it has one, in
    /// its stored record: what a test server's `--set-progression
    /// CHARACTER N` and stdin line do (contracts 10.1). A character that
    /// has never been played has only a row; its first save seeds the set
    /// progression into the new record ([`Accounts::saved`]).
    pub fn set_progression(&self, name: &str, progression: u8) -> Result<(), String> {
        if progression > 0x1F {
            return Err("the progression must be 0 to 31".into());
        }
        let key = rules::name_key(name);
        let db = self.inner.db.lock().unwrap_or_else(|p| p.into_inner());
        let row = db
            .character(&key)
            .map_err(|e| e.to_string())?
            .filter(|r| r.deleted.is_none())
            .ok_or_else(|| format!("no character named `{name}`"))?;
        let status = (row.summary.status & !(0x1F << 8)) | (u16::from(progression) << 8);
        let file = self.file_of(AccountId(row.account_id), &row.summary.name);
        if file.exists() {
            let store = CharacterStore::open(file.parent().unwrap_or(Path::new(".")))
                .map_err(|e| e.to_string())?;
            let mut record = store.load(&row.summary.name).map_err(|e| e.to_string())?;
            if !summary::set_status(&mut record, status) {
                return Err(format!("{}: the record has no status word", file.display()));
            }
            store
                .save(&row.summary.name, &record)
                .map_err(|e| e.to_string())?;
        }
        let saved = CharSummary {
            status,
            difficulties: status::difficulties(status),
            ..row.summary
        };
        if !db
            .save_summary(row.account_id, &saved)
            .map_err(|e| e.to_string())?
        {
            return Err(format!("no live character named `{name}`"));
        }
        self.log(&format!(
            "admin set the progression of `{}` to {progression}",
            saved.name
        ));
        Ok(())
    }

    /// Rebuilds the character rows from the character files. Rows of
    /// characters that were never played (no file yet) stay as they are.
    pub fn reindex(&self) -> Result<Reindexed, String> {
        let mut out = Reindexed::default();
        let root = self.inner.cfg.characters_dir();
        let mut db = self.inner.db.lock().unwrap_or_else(|p| p.into_inner());
        for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
            let dir = entry.map_err(|e| e.to_string())?.path();
            let Some(id) = dir
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.parse::<i64>().ok())
            else {
                out.skipped
                    .push(format!("{}: not an account folder", dir.display()));
                continue;
            };
            if db.account_by_id(id).map_err(|e| e.to_string())?.is_none() {
                out.skipped
                    .push(format!("{}: no account {id}", dir.display()));
                continue;
            }
            let store = CharacterStore::open(&dir).map_err(|e| e.to_string())?;
            for name in store.list().map_err(|e| e.to_string())? {
                let file = dir.join(format!("{name}.{EXTENSION}"));
                let played = std::fs::metadata(&file)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                let s = match store.load(&name) {
                    Ok(record) => summary::of_record(&record, played),
                    Err(e) => {
                        out.skipped.push(format!("{}: {e}", file.display()));
                        continue;
                    }
                };
                let s = match s {
                    Ok(s) if s.name.eq_ignore_ascii_case(&name) => CharSummary {
                        name: name.clone(),
                        ..s
                    },
                    Ok(s) => {
                        out.skipped.push(format!(
                            "{}: the record names `{}`",
                            file.display(),
                            s.name
                        ));
                        continue;
                    }
                    Err(e) => {
                        out.skipped.push(format!("{}: {e}", file.display()));
                        continue;
                    }
                };
                if db
                    .upsert_from_file(id, &s, unix_now())
                    .map_err(|e| e.to_string())?
                {
                    out.rows += 1;
                } else {
                    out.skipped
                        .push(format!("{}: the name is another account's", file.display()));
                }
            }
        }
        self.log(&format!(
            "reindex: {} rows, {} files skipped",
            out.rows,
            out.skipped.len()
        ));
        Ok(out)
    }

    /// Writes a backup into the new folder `dest` while the server runs: a
    /// consistent copy of the database (`accounts.sqlite`) and every
    /// account's character files (`characters/<account id>/`), each
    /// checked before it is copied. The backup is usable only once
    /// [`crate::backup::COMPLETE`] is in it. Returns how many character
    /// files it holds.
    pub fn backup_into(&self, dest: &Path) -> Result<usize, String> {
        let root = self.inner.cfg.characters_dir();
        if let (Ok(d), Ok(r)) = (
            dest.parent().unwrap_or(Path::new(".")).canonicalize(),
            self.inner.cfg.data.canonicalize(),
        ) {
            if d.starts_with(&r) {
                return Err("the backup must be outside the data folder".into());
            }
        }
        std::fs::create_dir(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        {
            let db = self.inner.db.lock().unwrap_or_else(|p| p.into_inner());
            db.vacuum_into(&dest.join(DATABASE))
                .map_err(|e| e.to_string())?;
        }
        let mut count = 0;
        for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
            let dir = entry.map_err(|e| e.to_string())?.path();
            if !dir.is_dir() {
                continue;
            }
            let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let to = dest.join("characters").join(id);
            std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            let store = CharacterStore::open(&dir).map_err(|e| e.to_string())?;
            for name in store.list().map_err(|e| e.to_string())? {
                let file = format!("{name}.{EXTENSION}");
                let bytes = std::fs::read(dir.join(&file)).map_err(|e| e.to_string())?;
                characters::decode(&name, &bytes).map_err(|e| e.to_string())?;
                write_new(&to.join(file), &bytes)?;
                count += 1;
            }
        }
        let marker = dest.join(format!("{}.tmp", crate::backup::COMPLETE));
        write_new(&marker, b"D2RUST server backup v1\n")?;
        std::fs::rename(&marker, dest.join(crate::backup::COMPLETE)).map_err(|e| e.to_string())?;
        self.log(&format!(
            "backup {}: {count} character files",
            dest.display()
        ));
        Ok(count)
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
