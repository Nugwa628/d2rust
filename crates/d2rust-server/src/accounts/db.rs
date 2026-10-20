//! The accounts database: one SQLite file in WAL mode (contracts section
//! 4). It holds the accounts and a summary row per online character; a
//! character's record itself is its `.d2rc` file, which is the truth (the
//! row is rebuilt from it by a reindex).
//!
//! Every call here is blocking; [`super::Accounts`] runs them off the
//! network threads.

use std::path::Path;
use std::time::Duration;

use d2_formats::d2s::status;
use d2_net::msg::account::CharSummary;
use rusqlite::{params, Connection, OptionalExtension};

/// The schema version in `PRAGMA user_version`.
const SCHEMA: i64 = 1;

const CREATE: &str = "
CREATE TABLE IF NOT EXISTS accounts(
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    name_key TEXT NOT NULL UNIQUE,
    pw_hash TEXT NOT NULL,
    created INTEGER NOT NULL,
    last_login INTEGER,
    disabled INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS characters(
    name_key TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    class INTEGER NOT NULL,
    level INTEGER NOT NULL,
    status INTEGER NOT NULL,
    look BLOB NOT NULL,
    created INTEGER NOT NULL,
    last_played INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER
);
CREATE INDEX IF NOT EXISTS characters_account ON characters(account_id);
";

/// A database failure, as text for the log.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("accounts database: {0}")]
pub struct DbError(pub String);

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError(e.to_string())
    }
}

/// An account's login row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub id: i64,
    pub name: String,
    pub pw_hash: String,
    pub disabled: bool,
}

/// A character's row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharRow {
    pub account_id: i64,
    /// Unix seconds of the delete, if deleted.
    pub deleted: Option<i64>,
    pub summary: CharSummary,
}

/// The open database.
#[derive(Debug)]
pub struct Db {
    conn: Connection,
}

fn summary_of(row: &rusqlite::Row, at: usize) -> rusqlite::Result<CharSummary> {
    let status: u16 = row.get(at + 3)?;
    let last_played: i64 = row.get(at + 5)?;
    Ok(CharSummary {
        name: row.get(at)?,
        class: row.get(at + 1)?,
        level: row.get(at + 2)?,
        status,
        difficulties: status::difficulties(status),
        look: row.get(at + 4)?,
        last_played: u64::try_from(last_played).unwrap_or(0),
    })
}

/// The summary columns in the order [`summary_of`] reads them.
const SUMMARY_COLUMNS: &str = "name, class, level, status, look, last_played";

impl Db {
    /// Opens (creating if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        Self::setup(conn)
    }

    /// A database in memory (tests).
    pub fn in_memory() -> Result<Self, DbError> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(conn: Connection) -> Result<Self, DbError> {
        conn.busy_timeout(Duration::from_secs(5))?;
        // An in-memory database answers "memory"; a file one "wal".
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA {
            return Err(DbError(format!(
                "schema version {version} is newer than this server's {SCHEMA}"
            )));
        }
        conn.execute_batch(CREATE)?;
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA}"))?;
        Ok(Db { conn })
    }

    /// Adds an account; `Ok(None)` if the name (any case) is taken.
    pub fn insert_account(
        &self,
        name: &str,
        key: &str,
        pw_hash: &str,
        now: i64,
    ) -> Result<Option<i64>, DbError> {
        let r = self.conn.execute(
            "INSERT INTO accounts(name, name_key, pw_hash, created) VALUES (?1, ?2, ?3, ?4)",
            params![name, key, pw_hash, now],
        );
        match r {
            Ok(_) => Ok(Some(self.conn.last_insert_rowid())),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn account_by_key(&self, key: &str) -> Result<Option<AccountRow>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, pw_hash, disabled FROM accounts WHERE name_key = ?1",
                [key],
                |r| {
                    Ok(AccountRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        pw_hash: r.get(2)?,
                        disabled: r.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()?)
    }

    pub fn account_by_id(&self, id: i64) -> Result<Option<AccountRow>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, pw_hash, disabled FROM accounts WHERE id = ?1",
                [id],
                |r| {
                    Ok(AccountRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        pw_hash: r.get(2)?,
                        disabled: r.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_last_login(&self, id: i64, now: i64) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE accounts SET last_login = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    /// Replaces an account's password hash; `false` if no such account.
    pub fn set_password(&self, key: &str, pw_hash: &str) -> Result<bool, DbError> {
        Ok(self.conn.execute(
            "UPDATE accounts SET pw_hash = ?2 WHERE name_key = ?1",
            params![key, pw_hash],
        )? == 1)
    }

    /// Disables or enables an account; `false` if no such account.
    pub fn set_disabled(&self, key: &str, disabled: bool) -> Result<bool, DbError> {
        Ok(self.conn.execute(
            "UPDATE accounts SET disabled = ?2 WHERE name_key = ?1",
            params![key, i64::from(disabled)],
        )? == 1)
    }

    /// The account's characters that are not deleted, oldest first.
    pub fn characters(&self, account: i64) -> Result<Vec<CharSummary>, DbError> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {SUMMARY_COLUMNS} FROM characters
             WHERE account_id = ?1 AND deleted IS NULL ORDER BY created, rowid"
        ))?;
        let rows = st.query_map([account], |r| summary_of(r, 0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn count_characters(&self, account: i64) -> Result<usize, DbError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM characters WHERE account_id = ?1 AND deleted IS NULL",
            [account],
            |r| r.get(0),
        )?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// The row holding the name `key`, deleted or not.
    pub fn character(&self, key: &str) -> Result<Option<CharRow>, DbError> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT account_id, deleted, {SUMMARY_COLUMNS} FROM characters WHERE name_key = ?1"
                ),
                [key],
                |r| {
                    Ok(CharRow {
                        account_id: r.get(0)?,
                        deleted: r.get(1)?,
                        summary: summary_of(r, 2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Adds a character row. A deleted row of the same name whose delete is
    /// at or before `freed_before` is dropped first; `Ok(false)` if the
    /// name is still held.
    pub fn insert_character(
        &mut self,
        account: i64,
        s: &CharSummary,
        now: i64,
        freed_before: i64,
    ) -> Result<bool, DbError> {
        let key = super::rules::name_key(&s.name);
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM characters WHERE name_key = ?1 AND deleted IS NOT NULL AND deleted <= ?2",
            params![key, freed_before],
        )?;
        let r = tx.execute(
            "INSERT INTO characters(name_key, name, account_id, class, level, status, look, created, last_played)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                key,
                s.name,
                account,
                s.class,
                s.level,
                s.status,
                s.look,
                now,
                i64::try_from(s.last_played).unwrap_or(i64::MAX)
            ],
        );
        match r {
            Ok(_) => {
                tx.commit()?;
                Ok(true)
            }
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Marks the account's character `key` deleted; `false` if it has no
    /// such character.
    pub fn mark_deleted(&self, account: i64, key: &str, now: i64) -> Result<bool, DbError> {
        Ok(self.conn.execute(
            "UPDATE characters SET deleted = ?3
             WHERE name_key = ?2 AND account_id = ?1 AND deleted IS NULL",
            params![account, key, now],
        )? == 1)
    }

    /// Writes a saved character's summary over its row; `false` if the
    /// account has no such live character.
    pub fn save_summary(&self, account: i64, s: &CharSummary) -> Result<bool, DbError> {
        Ok(self.conn.execute(
            "UPDATE characters SET class = ?3, level = ?4, status = ?5, look = ?6, last_played = ?7
             WHERE name_key = ?2 AND account_id = ?1 AND deleted IS NULL",
            params![
                account,
                super::rules::name_key(&s.name),
                s.class,
                s.level,
                s.status,
                s.look,
                i64::try_from(s.last_played).unwrap_or(i64::MAX)
            ],
        )? == 1)
    }

    /// Writes a row from a character file (reindex): updates the
    /// account's row of that name, or adds one. `false` if the name is
    /// held by another account.
    pub fn upsert_from_file(
        &mut self,
        account: i64,
        s: &CharSummary,
        now: i64,
    ) -> Result<bool, DbError> {
        if self.save_summary(account, s)? {
            return Ok(true);
        }
        let key = super::rules::name_key(&s.name);
        match self.character(&key)? {
            Some(row) if row.account_id != account => Ok(false),
            // This account's deleted row: the file came back, so the
            // character is live again.
            Some(_) => {
                self.conn.execute(
                    "UPDATE characters SET deleted = NULL WHERE name_key = ?1",
                    [&key],
                )?;
                self.save_summary(account, s)
            }
            None => self.insert_character(account, s, now, i64::MIN),
        }
    }

    /// Writes a consistent copy of the database to `path` (which must not
    /// exist), while the server runs.
    pub fn vacuum_into(&self, path: &Path) -> Result<(), DbError> {
        let p = path
            .to_str()
            .ok_or_else(|| DbError("the backup path is not text".into()))?;
        self.conn.execute("VACUUM INTO ?1", [p])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(name: &str) -> CharSummary {
        CharSummary {
            name: name.into(),
            class: 1,
            level: 1,
            status: status::EXPANSION,
            difficulties: 1,
            look: vec![0xFF; 32],
            last_played: 0,
        }
    }

    #[test]
    fn account_names_are_unique_in_any_case() {
        let db = Db::in_memory().unwrap();
        let id = db
            .insert_account("Mogger", "mogger", "h", 1)
            .unwrap()
            .unwrap();
        assert_eq!(db.insert_account("MOGGER", "mogger", "h", 2).unwrap(), None);
        let row = db.account_by_key("mogger").unwrap().unwrap();
        assert_eq!(
            (row.id, row.name.as_str(), row.disabled),
            (id, "Mogger", false)
        );
        assert!(db.set_disabled("mogger", true).unwrap());
        assert!(db.account_by_id(id).unwrap().unwrap().disabled);
        assert!(!db.set_disabled("nobody", true).unwrap());
        assert!(db.set_password("mogger", "h2").unwrap());
        assert_eq!(db.account_by_key("mogger").unwrap().unwrap().pw_hash, "h2");
    }

    #[test]
    fn character_rows_follow_their_account() {
        let mut db = Db::in_memory().unwrap();
        let a = db.insert_account("A1", "a1", "h", 1).unwrap().unwrap();
        let b = db.insert_account("B1", "b1", "h", 1).unwrap().unwrap();
        assert!(db.insert_character(a, &summary("Kashya"), 10, 0).unwrap());
        assert!(db.insert_character(a, &summary("Akara"), 11, 0).unwrap());
        assert!(!db.insert_character(b, &summary("KASHYA"), 12, 0).unwrap());
        let names: Vec<_> = db
            .characters(a)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Kashya", "Akara"]);
        assert!(db.characters(b).unwrap().is_empty());
        assert_eq!(db.count_characters(a).unwrap(), 2);

        let mut saved = summary("kashya");
        saved.level = 30;
        saved.status = status::EXPANSION | (5 << 8);
        assert!(db.save_summary(a, &saved).unwrap());
        assert!(!db.save_summary(b, &saved).unwrap());
        let row = db.character("kashya").unwrap().unwrap();
        assert_eq!(row.summary.name, "Kashya", "the stored case stays");
        assert_eq!(row.summary.level, 30);
        assert_eq!(row.summary.difficulties, 0b011);
    }

    #[test]
    fn a_deleted_name_is_held_until_freed() {
        let mut db = Db::in_memory().unwrap();
        let a = db.insert_account("A1", "a1", "h", 1).unwrap().unwrap();
        let b = db.insert_account("B1", "b1", "h", 1).unwrap().unwrap();
        db.insert_character(a, &summary("Kashya"), 10, 0).unwrap();
        assert!(!db.mark_deleted(b, "kashya", 100).unwrap());
        assert!(db.mark_deleted(a, "kashya", 100).unwrap());
        assert!(db.characters(a).unwrap().is_empty());
        assert_eq!(db.character("kashya").unwrap().unwrap().deleted, Some(100));
        // Still held when only deletes before 99 are freed.
        assert!(!db.insert_character(b, &summary("Kashya"), 200, 99).unwrap());
        // Freed once its delete is old enough.
        assert!(db
            .insert_character(b, &summary("Kashya"), 200, 100)
            .unwrap());
        assert_eq!(db.character("kashya").unwrap().unwrap().account_id, b);
    }

    #[test]
    fn reindex_rows() {
        let mut db = Db::in_memory().unwrap();
        let a = db.insert_account("A1", "a1", "h", 1).unwrap().unwrap();
        let b = db.insert_account("B1", "b1", "h", 1).unwrap().unwrap();
        let mut s = summary("Kashya");
        assert!(db.upsert_from_file(a, &s, 5).unwrap());
        s.level = 9;
        assert!(db.upsert_from_file(a, &s, 6).unwrap());
        assert_eq!(db.character("kashya").unwrap().unwrap().summary.level, 9);
        assert!(!db.upsert_from_file(b, &s, 7).unwrap());
        db.mark_deleted(a, "kashya", 8).unwrap();
        assert!(db.upsert_from_file(a, &s, 9).unwrap());
        assert_eq!(db.character("kashya").unwrap().unwrap().deleted, None);
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let dir = std::env::temp_dir().join(format!("d2rust-accounts-db-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.sqlite");
        drop(Db::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA user_version = 99")
            .unwrap();
        assert!(Db::open(&path).unwrap_err().0.contains("newer"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
