//! Hub storage (ADR-0014): an append-only event log with exactly-once application per `(producer, seq)`,
//! plus key-value settings. Embedded SQLite in WAL mode. The trait avoids SQLite-only concepts so a
//! Postgres backend can implement it later (Phase 7).
//!
//! Protocol rule (docs/specs/protocol.md §3): a producer numbers its events 1, 2, 3, …; the hub applies each
//! at most once and acknowledges the highest contiguous number.

use std::fmt;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

/// Outcome of [`EventStore::append`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Append {
    /// Stored; `pos` is its position in the global log (1, 2, 3, …).
    Applied { pos: u64 },
    /// Already applied earlier; nothing changed. The producer may drop it.
    Duplicate,
    /// Not the next event from this producer; `expected` is the one the hub is waiting for.
    Gap { expected: u64 },
    /// Stored as the producer's new base after it said it cannot replay (ADR-0054): `from_seq..=to_seq` were skipped and are
    /// recorded as lost. Only [`EventStore::append_resync`] gives this.
    Resynced {
        pos: u64,
        from_seq: u64,
        to_seq: u64,
    },
}

/// One event as stored: opaque payload (an encoded protocol `Event`) plus its coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub pos: u64,
    pub producer: String,
    pub seq: u64,
    pub at_ms: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct StoreError(pub(crate) String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "store: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self(e.to_string())
    }
}

pub trait EventStore {
    /// Applies the producer's next event, or reports why it was not applied.
    fn append(
        &self,
        producer: &str,
        seq: u64,
        at_ms: u64,
        payload: &[u8],
    ) -> Result<Append, StoreError>;
    /// Like [`append`](Self::append), for the producer's first event after it said it cannot replay (`SnapshotRequired`, ADR-0054):
    /// an event above the next number is stored too, and in the same transaction the producer's numbering moves to it and the
    /// skipped numbers are recorded as lost, at `resync_at_ms`. Never backwards: at or below what is held is a `Duplicate`.
    fn append_resync(
        &self,
        producer: &str,
        seq: u64,
        at_ms: u64,
        payload: &[u8],
        resync_at_ms: u64,
    ) -> Result<Append, StoreError>;
    /// Highest contiguous `seq` applied for the producer (0 when none): the acknowledgement to send back.
    fn acked(&self, producer: &str) -> Result<u64, StoreError>;
    /// Events with a global position greater than `after`, oldest first, at most `limit`.
    fn events_after(&self, after: u64, limit: usize) -> Result<Vec<StoredEvent>, StoreError>;
    fn setting(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn set_setting(&self, key: &str, value: &[u8]) -> Result<(), StoreError>;
}

const SCHEMA_VERSION: i64 = 22;

/// A transaction that reads and then writes, begun with the write lock (`BEGIN IMMEDIATE`). Other processes write to the hub's
/// database too (the front door and the terminal client make sign-in codes, `agora hub enroll` makes codes): a deferred transaction
/// that has read cannot wait for the write lock, so SQLite answers "database is locked" at once instead of waiting the busy timeout,
/// and the hub dropped a node's connection over it (#935). Taken first, the lock is waited for like any single write.
pub(crate) fn write_tx(conn: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        // WAL: readers do not block the single writer, and a crash never leaves a half-written page.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    /// The total size of the stored payloads after `after`, without reading them: SQLite answers `length()` of a blob from the
    /// record header (a replay decides from this whether to read the events at all, before any is copied into memory).
    pub fn payload_bytes_after(&self, after: u64) -> Result<u64, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(SUM(length(payload)), 0) FROM events WHERE pos > ?1",
        )?;
        let total: i64 = stmt.query_row(params![to_i64(after)?], |r| r.get(0))?;
        Ok(to_u64(total))
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError(format!(
                "database schema {version} is newer than this hub understands ({SCHEMA_VERSION})"
            )));
        }
        if version < 1 {
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE events (
                     pos      INTEGER PRIMARY KEY AUTOINCREMENT,
                     producer TEXT    NOT NULL,
                     seq      INTEGER NOT NULL,
                     at_ms    INTEGER NOT NULL,
                     payload  BLOB    NOT NULL,
                     UNIQUE (producer, seq)
                 );
                 CREATE TABLE producers (
                     producer TEXT PRIMARY KEY,
                     last_seq INTEGER NOT NULL
                 );
                 CREATE TABLE settings (
                     key   TEXT PRIMARY KEY,
                     value BLOB NOT NULL
                 );
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        if version < 2 {
            // Device enrollment (ADR-0015): one-time codes (hashed) and the devices they enrolled.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE enrollment_codes (
                     code_hash  BLOB PRIMARY KEY,
                     expires_ms INTEGER NOT NULL
                 );
                 CREATE TABLE devices (
                     serial     TEXT PRIMARY KEY,
                     node_id    TEXT NOT NULL,
                     enrolled_ms INTEGER NOT NULL,
                     revoked_ms INTEGER
                 );
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
        }
        if version < 3 {
            // Client sign-in: one-time codes and the sessions they were exchanged for (both hashed).
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE signin_codes (
                     code_hash  BLOB PRIMARY KEY,
                     expires_ms INTEGER NOT NULL
                 );
                 CREATE TABLE sessions (
                     token_hash BLOB PRIMARY KEY,
                     created_ms INTEGER NOT NULL,
                     expires_ms INTEGER NOT NULL
                 );
                 PRAGMA user_version = 3;
                 COMMIT;",
            )?;
        }
        if version < 4 {
            // Passkeys (WebAuthn, ADR-0015): the public half only; the private key never leaves the user's authenticator.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE passkeys (
                     id         TEXT PRIMARY KEY,
                     public_key BLOB NOT NULL,
                     sign_count INTEGER NOT NULL,
                     created_ms INTEGER NOT NULL
                 );
                 PRAGMA user_version = 4;
                 COMMIT;",
            )?;
        }
        if version < 5 {
            // Team mode (ADR-0049): people with a role. A session or sign-in code with no user belongs to the owner, as before.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE users (
                     id         TEXT PRIMARY KEY,
                     name       TEXT NOT NULL,
                     role       TEXT NOT NULL,
                     created_ms INTEGER NOT NULL
                 );
                 ALTER TABLE signin_codes ADD COLUMN user_id TEXT;
                 ALTER TABLE sessions ADD COLUMN user_id TEXT;
                 PRAGMA user_version = 5;
                 COMMIT;",
            )?;
        }
        if version < 6 {
            // The audit trail (ADR-0049): who did what, where, and how it ended. Rows are never deleted and only the outcome of a row
            // still marked `sent` may change, once; the triggers hold that even for a bug elsewhere in the hub.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE audit (
                     id      INTEGER PRIMARY KEY AUTOINCREMENT,
                     at_ms   INTEGER NOT NULL,
                     who     TEXT NOT NULL,
                     action  TEXT NOT NULL,
                     node    TEXT NOT NULL,
                     outcome TEXT NOT NULL
                 );
                 CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit
                 BEGIN SELECT RAISE(ABORT, 'the audit trail is append-only'); END;
                 CREATE TRIGGER audit_only_the_outcome BEFORE UPDATE ON audit
                 WHEN OLD.outcome != 'sent' OR NEW.id != OLD.id OR NEW.at_ms != OLD.at_ms OR NEW.who != OLD.who
                      OR NEW.action != OLD.action OR NEW.node != OLD.node
                 BEGIN SELECT RAISE(ABORT, 'the audit trail is append-only'); END;
                 PRAGMA user_version = 6;
                 COMMIT;",
            )?;
        }
        if version < 7 {
            // Which projects a member or viewer may see and work in (ADR-0049): `node/project`, one row each. An owner needs none.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE grants (
                     user_id TEXT NOT NULL,
                     node    TEXT NOT NULL,
                     project TEXT NOT NULL,
                     PRIMARY KEY (user_id, node, project)
                 );
                 PRAGMA user_version = 7;
                 COMMIT;",
            )?;
        }
        if version < 8 {
            // A passkey belongs to the person who added it, so signing in with it gives that person's session and nobody else's
            // (ADR-0049). A passkey with no person is the owner's, as before.
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE passkeys ADD COLUMN user_id TEXT;
                 PRAGMA user_version = 8;
                 COMMIT;",
            )?;
        }
        if version < 9 {
            // Numbers a node could no longer replay and the hub skipped (ADR-0054): the loss is recorded, never silent.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE resyncs (
                     id       INTEGER PRIMARY KEY AUTOINCREMENT,
                     producer TEXT    NOT NULL,
                     from_seq INTEGER NOT NULL,
                     to_seq   INTEGER NOT NULL,
                     at_ms    INTEGER NOT NULL
                 );
                 PRAGMA user_version = 9;
                 COMMIT;",
            )?;
        }
        if version < 10 {
            // Which session did it (L16): a short id of the session, so two windows of the same person can be told apart. Rows
            // written before have none. The trigger is made again so the new column is as fixed as the others.
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE audit ADD COLUMN session TEXT NOT NULL DEFAULT '';
                 DROP TRIGGER audit_only_the_outcome;
                 CREATE TRIGGER audit_only_the_outcome BEFORE UPDATE ON audit
                 WHEN OLD.outcome != 'sent' OR NEW.id != OLD.id OR NEW.at_ms != OLD.at_ms OR NEW.who != OLD.who
                      OR NEW.action != OLD.action OR NEW.node != OLD.node OR NEW.session != OLD.session
                 BEGIN SELECT RAISE(ABORT, 'the audit trail is append-only'); END;
                 PRAGMA user_version = 10;
                 COMMIT;",
            )?;
        }
        if version < 11 {
            // ADR-0061: each person's registered devices (candidates a machine may come to trust; never trust by themselves), and the
            // endorsements and revocations the hub carries to machines, kept so a machine that connects later still gets them. The
            // hub cannot make either: they are signed by a device and verified on the machine.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS user_devices (
                     device_id         TEXT PRIMARY KEY,
                     user_id           TEXT NOT NULL,
                     name              TEXT NOT NULL,
                     public_key        BLOB NOT NULL,
                     created_ms        INTEGER NOT NULL,
                     last_used_ms      INTEGER NOT NULL,
                     approval_asked_ms INTEGER NOT NULL DEFAULT 0,
                     revoked_ms        INTEGER NOT NULL DEFAULT 0,
                     signed_at_ms      INTEGER NOT NULL DEFAULT 0,
                     signature         BLOB NOT NULL DEFAULT x''
                 );
                 CREATE TABLE IF NOT EXISTS device_endorsements (
                     id         INTEGER PRIMARY KEY AUTOINCREMENT,
                     device_id  TEXT NOT NULL,
                     endorser   TEXT NOT NULL,
                     issued_ms  INTEGER NOT NULL,
                     expires_ms INTEGER NOT NULL,
                     signature  BLOB NOT NULL UNIQUE,
                     payload    BLOB NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS device_revocations (
                     id        INTEGER PRIMARY KEY AUTOINCREMENT,
                     device_id TEXT NOT NULL,
                     revoker   TEXT NOT NULL,
                     issued_ms INTEGER NOT NULL,
                     signature BLOB NOT NULL UNIQUE,
                     payload   BLOB NOT NULL
                 );
                 PRAGMA user_version = 11;
                 COMMIT;",
            )?;
        }
        if version < 12 {
            // ADR-0076: tokens that can only read `GET /api/metrics`. Only the SHA-256 of a token is kept, apart from sessions.
            // `expires_ms` and `revoked_ms` are 0 for "never" and "not revoked".
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS metrics_tokens (
                     name         TEXT PRIMARY KEY,
                     token_hash   BLOB NOT NULL UNIQUE,
                     created_ms   INTEGER NOT NULL,
                     expires_ms   INTEGER NOT NULL DEFAULT 0,
                     last_used_ms INTEGER NOT NULL DEFAULT 0,
                     revoked_ms   INTEGER NOT NULL DEFAULT 0
                 );
                 PRAGMA user_version = 12;
                 COMMIT;",
            )?;
        }
        // ADR-0085 (schema 13): per certificate, when a machine first connected with it (NULL until it does) so the hub can revoke a
        // machine's older certificates a week after it connected with a newer one; and, for a certificate issued by a renewal, its PEM
        // (public data) and the SHA-256 of the public key of the request it was issued for, so a renewal whose answer was lost can be
        // answered again with the same certificate. Checked at EVERY open, not only when the version is older: an unreleased build of
        // schema 13 (developer or test hubs) left databases at version 13 without the later columns, and every renewal on one would fail.
        // Each column is added only when missing, inside one transaction, so it is idempotent (a database taken back to an older version
        // by hand, as the tests do, has some of them already).
        let mut script = String::new();
        for (name, kind) in [
            ("first_seen_ms", "INTEGER"),
            ("cert_pem", "TEXT"),
            ("csr_key_hash", "BLOB"),
        ] {
            let has: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('devices') WHERE name = ?1",
                [name],
                |r| r.get(0),
            )?;
            if has == 0 {
                script.push_str(&format!(" ALTER TABLE devices ADD COLUMN {name} {kind};"));
            }
        }
        if version < 13 {
            script.push_str(" PRAGMA user_version = 13;");
        }
        if !script.is_empty() {
            conn.execute_batch(&format!("BEGIN;{script} COMMIT;"))?;
        }
        if version < 14 {
            let mut script = String::new();
            for (name, kind) in [
                ("project_id", "TEXT NOT NULL DEFAULT ''"),
                ("agent_id", "TEXT NOT NULL DEFAULT ''"),
                ("requested_device_id", "TEXT NOT NULL DEFAULT ''"),
                (
                    "signer_verified",
                    "INTEGER NOT NULL DEFAULT 0 CHECK(signer_verified IN (0, 1))",
                ),
            ] {
                let has: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('audit') WHERE name = ?1",
                    [name],
                    |r| r.get(0),
                )?;
                if has == 0 {
                    script.push_str(&format!(" ALTER TABLE audit ADD COLUMN {name} {kind};"));
                }
            }
            conn.execute_batch(&format!(
                "BEGIN;{script}
                 DROP TRIGGER audit_only_the_outcome;
                 CREATE TRIGGER audit_only_the_outcome BEFORE UPDATE ON audit
                 WHEN OLD.outcome != 'sent' OR NEW.id != OLD.id OR NEW.at_ms != OLD.at_ms OR NEW.who != OLD.who
                      OR NEW.action != OLD.action OR NEW.node != OLD.node OR NEW.session != OLD.session
                      OR NEW.project_id != OLD.project_id OR NEW.agent_id != OLD.agent_id
                      OR NEW.requested_device_id != OLD.requested_device_id
                      OR (NEW.signer_verified != OLD.signer_verified AND
                          (NEW.signer_verified != 1 OR NEW.outcome != 'ok' OR OLD.requested_device_id = ''))
                 BEGIN SELECT RAISE(ABORT, 'the audit trail is append-only'); END;
                 PRAGMA user_version = 14;
                 COMMIT;"
            ))?;
        }
        if version < 15 {
            // ADR-0113 decision 6: the community theme catalog the owner fetched (`agora hub themes update`), one row per accepted
            // theme with its file as fetched. An update replaces every row in one transaction.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS theme_catalog (
                     id         TEXT PRIMARY KEY,
                     name       TEXT NOT NULL,
                     version    INTEGER NOT NULL,
                     author     TEXT NOT NULL,
                     licence    TEXT NOT NULL,
                     sha256     TEXT NOT NULL,
                     data       TEXT NOT NULL
                 );
                 PRAGMA user_version = 15;
                 COMMIT;",
            )?;
        }
        if version < 16 {
            // ADR-0114 decision 6: the plugins the owner installed on this hub (each package as opaque bytes, never run, with what the
            // hub read from its checked manifest) and what each person allowed each plugin to see, for the version they were shown.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS plugins (
                     id           TEXT PRIMARY KEY,
                     version      TEXT NOT NULL,
                     name         TEXT NOT NULL,
                     capabilities TEXT NOT NULL,
                     sha256       TEXT NOT NULL,
                     installed_ms INTEGER NOT NULL,
                     package      BLOB NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS plugin_grants (
                     user_id   TEXT NOT NULL,
                     plugin_id TEXT NOT NULL,
                     version   TEXT NOT NULL,
                     seen      TEXT NOT NULL,
                     granted   TEXT NOT NULL,
                     set_ms    INTEGER NOT NULL,
                     PRIMARY KEY (user_id, plugin_id)
                 );
                 PRAGMA user_version = 16;
                 COMMIT;",
            )?;
        }
        if version < 17 {
            // ADR-0113 decision 6 (step 4): each person's own settings values, opaque bytes the hub never reads (`user_settings.rs`), with
            // a version raised on every write so two windows never overwrite each other unseen, and each person's random tag (a browser
            // tells by it whose value it last synced with).
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS user_settings (
                     user_id    TEXT NOT NULL,
                     key        TEXT NOT NULL,
                     version    INTEGER NOT NULL,
                     value      BLOB NOT NULL,
                     updated_ms INTEGER NOT NULL,
                     PRIMARY KEY (user_id, key)
                 );
                 CREATE TABLE IF NOT EXISTS user_settings_tags (
                     user_id TEXT PRIMARY KEY,
                     tag     TEXT NOT NULL
                 );
                 PRAGMA user_version = 17;
                 COMMIT;",
            )?;
        }
        if version < 18 {
            // ADR-0114 decision 7 (step 4): the community plugin catalog's entries, as its last signed index listed them
            // (`plugin_catalog.rs`). The archives are not kept here: one is fetched only when the owner installs it, and kept in `plugins`.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS plugin_catalog (
                     id           TEXT PRIMARY KEY,
                     name         TEXT NOT NULL,
                     version      TEXT NOT NULL,
                     author       TEXT NOT NULL,
                     licence      TEXT NOT NULL,
                     description  TEXT NOT NULL,
                     capabilities TEXT NOT NULL,
                     sha256       TEXT NOT NULL,
                     size         INTEGER NOT NULL,
                     file         TEXT NOT NULL
                 );
                 PRAGMA user_version = 18;
                 COMMIT;",
            )?;
        }
        if version < 19 {
            // ADR-0120: the day a catalog first listed an entry and the popularity it publishes (both catalogs), and the plugins'
            // screenshots as the hub fetched and checked them. Replaced with their catalog in one transaction by each update. A
            // column already there (a database whose version number was set back) is not added again.
            let mut script = String::new();
            for table in ["theme_catalog", "plugin_catalog"] {
                for (name, kind) in [
                    ("added", "TEXT NOT NULL DEFAULT ''"),
                    ("popularity", "INTEGER NOT NULL DEFAULT 0"),
                ] {
                    let has: i64 = conn.query_row(
                        &format!(
                            "SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"
                        ),
                        [name],
                        |r| r.get(0),
                    )?;
                    if has == 0 {
                        script.push_str(&format!(" ALTER TABLE {table} ADD COLUMN {name} {kind};"));
                    }
                }
            }
            conn.execute_batch(&format!(
                "BEGIN;{script}
                 CREATE TABLE IF NOT EXISTS plugin_catalog_screenshot (
                     id         TEXT NOT NULL,
                     n          INTEGER NOT NULL,
                     media_type TEXT NOT NULL,
                     data       BLOB NOT NULL,
                     alt        TEXT NOT NULL,
                     PRIMARY KEY (id, n)
                 );
                 PRAGMA user_version = 19;
                 COMMIT;"
            ))?;
        }
        if version < 20 {
            // ADR-0122: the owner's newest signed word on each shared item's name (a removal included, `kind` 0, so a node can be
            // handed the word that ended an item), and what each machine said when it refused one.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS shared_items (
                     name      TEXT PRIMARY KEY,
                     kind      INTEGER NOT NULL,
                     item_hash TEXT NOT NULL,
                     issued_ms INTEGER NOT NULL,
                     signer    TEXT NOT NULL,
                     signed    BLOB NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS shared_item_refusals (
                     name      TEXT NOT NULL,
                     node_id   TEXT NOT NULL,
                     item_hash TEXT NOT NULL,
                     code      TEXT NOT NULL,
                     reason    TEXT NOT NULL,
                     at_ms     INTEGER NOT NULL,
                     PRIMARY KEY (name, node_id)
                 );
                 PRAGMA user_version = 20;
                 COMMIT;",
            )?;
        }
        if version < 21 {
            // ADR-0125: old hubs must not open a board with a pending review as a normal claim.
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS task_review_projects (project_id TEXT PRIMARY KEY);
                 PRAGMA user_version = 21;
                 COMMIT;",
            )?;
        }
        if version < 22 {
            // ADR-0126: earlier hubs would strip parent links when rewriting a stored board.
            conn.pragma_update(None, "user_version", 22)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// How many lost ranges resyncs recorded (counted by the database, no rows read).
    pub fn resync_count(&self) -> Result<usize, StoreError> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM resyncs", [], |r| r.get(0))?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// The recorded resyncs (ADR-0054), oldest first: `(producer, from_seq, to_seq, at_ms)`.
    pub fn resyncs(&self) -> Result<Vec<(String, u64, u64, u64)>, StoreError> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT producer, from_seq, to_seq, at_ms FROM resyncs ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                to_u64(r.get(1)?),
                to_u64(r.get(2)?),
                to_u64(r.get(3)?),
            ))
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    /// Appends `seq` for `producer`. With `resync_at_ms`, an event above the next number becomes the new base (ADR-0054).
    fn append_inner(
        &self,
        producer: &str,
        seq: u64,
        at_ms: u64,
        payload: &[u8],
        resync_at_ms: Option<u64>,
    ) -> Result<Append, StoreError> {
        let (seq_i, at_i) = (to_i64(seq)?, to_i64(at_ms)?);
        let mut conn = self.lock()?;
        let tx = write_tx(&mut conn)?;
        let last: i64 = tx
            .query_row(
                "SELECT last_seq FROM producers WHERE producer = ?1",
                [producer],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if seq_i <= last {
            return Ok(Append::Duplicate);
        }
        let Some(expected) = last.checked_add(1) else {
            return Ok(Append::Duplicate);
        };
        if seq_i != expected && resync_at_ms.is_none() {
            return Ok(Append::Gap {
                expected: to_u64(expected),
            });
        }
        // Only a real jump is a resync; the next number is an ordinary append even right after `SnapshotRequired`.
        let skipped = resync_at_ms.filter(|_| seq_i != expected);
        tx.execute(
            "INSERT INTO events (producer, seq, at_ms, payload) VALUES (?1, ?2, ?3, ?4)",
            params![producer, seq_i, at_i, payload],
        )?;
        let pos = to_u64(tx.last_insert_rowid());
        tx.execute(
            "INSERT INTO producers (producer, last_seq) VALUES (?1, ?2)
             ON CONFLICT(producer) DO UPDATE SET last_seq = excluded.last_seq",
            params![producer, seq_i],
        )?;
        if let Some(when) = skipped {
            tx.execute(
                "INSERT INTO resyncs (producer, from_seq, to_seq, at_ms) VALUES (?1, ?2, ?3, ?4)",
                params![producer, expected, seq_i - 1, to_i64(when)?],
            )?;
        }
        tx.commit()?;
        Ok(match skipped {
            Some(_) => Append::Resynced {
                pos,
                from_seq: to_u64(expected),
                to_seq: seq - 1,
            },
            None => Append::Applied { pos },
        })
    }

    /// The hub's own events, numbered from `first_seq`, and one setting, in one transaction: all of them are stored or none is (a
    /// board change and its audit events, #517 review). Refused when the producer's numbering is not at `first_seq - 1`. Returns
    /// each event's position, in order.
    pub fn append_batch_with_setting(
        &self,
        producer: &str,
        first_seq: u64,
        events: &[(u64, Vec<u8>)],
        setting: Option<(&str, &[u8])>,
    ) -> Result<Vec<u64>, StoreError> {
        self.append_batch_with_settings(producer, first_seq, events, setting.as_slice())
    }

    /// Removes the setting `key`; one that is not there is not an error (#1150: a list of owed notices given or no longer owed
    /// leaves no row behind).
    pub fn remove_setting(&self, key: &str) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }

    /// Local hub administration requests Review for a shared project.
    pub fn enable_task_review(&self, project: &str) -> Result<(), StoreError> {
        self.lock()?.execute(
            "INSERT OR IGNORE INTO task_review_projects (project_id) VALUES (?1)",
            [project],
        )?;
        Ok(())
    }

    pub(crate) fn requested_review_projects(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.lock()?;
        let mut query =
            conn.prepare("SELECT project_id FROM task_review_projects ORDER BY project_id")?;
        let projects = query.query_map([], |row| row.get(0))?;
        Ok(projects.collect::<Result<_, _>>()?)
    }

    pub(crate) fn handoff_settings(&self) -> Result<Vec<Vec<u8>>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key GLOB 'task-handoff.*' ORDER BY key")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// [`append_batch_with_setting`](Self::append_batch_with_setting) with any number of settings (ADR-0115: a merged board and the
    /// markers of the boards merged into it, #1150), all in the one transaction.
    pub fn append_batch_with_settings(
        &self,
        producer: &str,
        first_seq: u64,
        events: &[(u64, Vec<u8>)],
        settings: &[(&str, &[u8])],
    ) -> Result<Vec<u64>, StoreError> {
        self.append_batch_with_setting_changes(producer, first_seq, events, settings, None)
    }

    pub(crate) fn append_batch_with_setting_changes(
        &self,
        producer: &str,
        first_seq: u64,
        events: &[(u64, Vec<u8>)],
        settings: &[(&str, &[u8])],
        remove: Option<&str>,
    ) -> Result<Vec<u64>, StoreError> {
        let mut conn = self.lock()?;
        let tx = write_tx(&mut conn)?;
        let last: i64 = tx
            .query_row(
                "SELECT last_seq FROM producers WHERE producer = ?1",
                [producer],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if to_u64(last).checked_add(1) != Some(first_seq) {
            return Err(StoreError(format!(
                "{producer} is numbered at {last}, not {}",
                first_seq.saturating_sub(1)
            )));
        }
        let mut positions = Vec::with_capacity(events.len());
        let mut seq = first_seq;
        for (at_ms, payload) in events {
            tx.execute(
                "INSERT INTO events (producer, seq, at_ms, payload) VALUES (?1, ?2, ?3, ?4)",
                params![producer, to_i64(seq)?, to_i64(*at_ms)?, payload],
            )?;
            positions.push(to_u64(tx.last_insert_rowid()));
            seq += 1;
        }
        if !events.is_empty() {
            tx.execute(
                "INSERT INTO producers (producer, last_seq) VALUES (?1, ?2)
                 ON CONFLICT(producer) DO UPDATE SET last_seq = excluded.last_seq",
                params![producer, to_i64(seq - 1)?],
            )?;
        }
        for (key, value) in settings {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
        if let Some(key) = remove {
            tx.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        }
        tx.commit()?;
        Ok(positions)
    }

    /// Online, consistent backup into a new file (`VACUUM INTO` takes a snapshot while writers keep working).
    /// Refuses to overwrite: a backup path that exists is almost certainly a mistake.
    pub fn backup_to(&self, dest: &Path) -> Result<(), StoreError> {
        if dest.exists() {
            return Err(StoreError(format!("{} already exists", dest.display())));
        }
        let conn = self.lock()?;
        let path = dest
            .to_str()
            .ok_or_else(|| StoreError("backup path is not valid UTF-8".into()))?;
        conn.execute("VACUUM INTO ?1", [path])?;
        Ok(())
    }

    pub(crate) fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StoreError> {
        self.conn
            .lock()
            .map_err(|_| StoreError("store lock poisoned".into()))
    }
}

/// SQLite stores integers as i64; sequence numbers beyond that are not representable and are refused.
fn to_i64(v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| StoreError(format!("{v} out of range")))
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// Checks that `path` is a healthy hub database this build can read: SQLite integrity check passes and the
/// schema is not newer than ours.
pub fn verify(path: &Path) -> Result<(), StoreError> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(StoreError(format!("integrity check failed: {check}")));
    }
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(StoreError(format!("unsupported schema version {version}")));
    }
    Ok(())
}

/// Replaces the database at `dest` with the verified backup `from`. The hub must not have `dest` open. The old
/// file is kept next to it as `<dest>.before-restore` so a wrong restore can be undone, with its write-ahead log as
/// `<dest>.before-restore-wal` when it has one (after a crash it holds the file's last commits).
pub fn restore(from: &Path, dest: &Path) -> Result<(), StoreError> {
    verify(from)?;
    let io = |e: std::io::Error| StoreError(e.to_string());
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dest.with_file_name(format!("{name}.restoring"));
    std::fs::copy(from, &tmp).map_err(io)?;
    if dest.exists() {
        let kept = format!("{name}.before-restore");
        std::fs::rename(dest, dest.with_file_name(&kept)).map_err(io)?;
        // After a crash the replaced database's last commits are only in its write-ahead log: it goes with the kept copy (and an
        // older kept copy's log, which would corrupt this one, goes away).
        for ext in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(dest.with_file_name(format!("{kept}{ext}")));
        }
        let wal = dest.with_file_name(format!("{name}-wal"));
        if wal.exists() {
            std::fs::rename(&wal, dest.with_file_name(format!("{kept}-wal"))).map_err(io)?;
        }
    }
    // Stale write-ahead files belong to the replaced database and would corrupt the restored one.
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(dest.with_file_name(format!("{name}{ext}")));
    }
    std::fs::rename(&tmp, dest).map_err(io)
}

impl EventStore for SqliteStore {
    fn append(
        &self,
        producer: &str,
        seq: u64,
        at_ms: u64,
        payload: &[u8],
    ) -> Result<Append, StoreError> {
        self.append_inner(producer, seq, at_ms, payload, None)
    }

    fn append_resync(
        &self,
        producer: &str,
        seq: u64,
        at_ms: u64,
        payload: &[u8],
        resync_at_ms: u64,
    ) -> Result<Append, StoreError> {
        self.append_inner(producer, seq, at_ms, payload, Some(resync_at_ms))
    }

    fn acked(&self, producer: &str) -> Result<u64, StoreError> {
        let conn = self.lock()?;
        let last: Option<i64> = conn
            .query_row(
                "SELECT last_seq FROM producers WHERE producer = ?1",
                [producer],
                |r| r.get(0),
            )
            .optional()?;
        Ok(last.map_or(0, to_u64))
    }

    fn events_after(&self, after: u64, limit: usize) -> Result<Vec<StoredEvent>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT pos, producer, seq, at_ms, payload FROM events WHERE pos > ?1 ORDER BY pos LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![to_i64(after)?, limit], |r| {
            Ok(StoredEvent {
                pos: to_u64(r.get(0)?),
                producer: r.get(1)?,
                seq: to_u64(r.get(2)?),
                at_ms: to_u64(r.get(3)?),
                payload: r.get(4)?,
            })
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    fn setting(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let conn = self.lock()?;
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(Into::into)
    }

    fn set_setting(&self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn store() -> SqliteStore {
        SqliteStore::open_in_memory().expect("open")
    }

    #[test]
    fn applies_in_order_and_acks_contiguously() {
        let s = store();
        assert_eq!(s.acked("n1").unwrap(), 0);
        assert_eq!(
            s.append("n1", 1, 10, b"a").unwrap(),
            Append::Applied { pos: 1 }
        );
        assert_eq!(
            s.append("n1", 2, 11, b"b").unwrap(),
            Append::Applied { pos: 2 }
        );
        assert_eq!(s.acked("n1").unwrap(), 2);
    }

    #[test]
    fn duplicate_is_ignored_and_gap_is_reported() {
        let s = store();
        s.append("n1", 1, 0, b"a").unwrap();
        assert_eq!(s.append("n1", 1, 0, b"other").unwrap(), Append::Duplicate);
        assert_eq!(
            s.append("n1", 3, 0, b"c").unwrap(),
            Append::Gap { expected: 2 }
        );
        // Nothing was stored for either.
        let all = s.events_after(0, 10).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].payload, b"a");
        assert_eq!(s.acked("n1").unwrap(), 1);
    }

    #[test]
    fn producers_are_numbered_independently_and_interleave_in_the_log() {
        let s = store();
        s.append("n1", 1, 0, b"a1").unwrap();
        s.append("n2", 1, 0, b"b1").unwrap();
        s.append("n1", 2, 0, b"a2").unwrap();
        let all = s.events_after(0, 10).unwrap();
        let order: Vec<_> = all.iter().map(|e| (e.producer.as_str(), e.seq)).collect();
        assert_eq!(order, [("n1", 1), ("n2", 1), ("n1", 2)]);
        assert_eq!(s.acked("n2").unwrap(), 1);
    }

    #[test]
    fn events_after_pages_by_position() {
        let s = store();
        for i in 1..=5 {
            s.append("n", i, 0, &[i as u8]).unwrap();
        }
        let first = s.events_after(0, 2).unwrap();
        assert_eq!(first.iter().map(|e| e.pos).collect::<Vec<_>>(), [1, 2]);
        let next = s.events_after(first[1].pos, 10).unwrap();
        assert_eq!(next.iter().map(|e| e.pos).collect::<Vec<_>>(), [3, 4, 5]);
        assert!(s.events_after(5, 10).unwrap().is_empty());
    }

    #[test]
    fn survives_reopen() {
        let dir = std::env::temp_dir().join(format!("agora-hub-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hub.db");
        {
            let s = SqliteStore::open(&path).unwrap();
            s.append("n1", 1, 5, b"x").unwrap();
            s.set_setting("k", b"v").unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(s.acked("n1").unwrap(), 1);
        assert_eq!(s.append("n1", 1, 5, b"x").unwrap(), Append::Duplicate);
        assert_eq!(s.setting("k").unwrap().as_deref(), Some(&b"v"[..]));
        assert_eq!(s.events_after(0, 10).unwrap()[0].at_ms, 5);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_overwrite_and_missing_is_none() {
        let s = store();
        assert_eq!(s.setting("a").unwrap(), None);
        s.set_setting("a", b"1").unwrap();
        s.set_setting("a", b"2").unwrap();
        assert_eq!(s.setting("a").unwrap().as_deref(), Some(&b"2"[..]));
    }

    #[test]
    fn refuses_a_newer_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(SqliteStore::init(conn).is_err());
    }

    #[test]
    fn concurrent_producers_stay_consistent() {
        let s = std::sync::Arc::new(store());
        let handles: Vec<_> = (0..4)
            .map(|p| {
                let s = s.clone();
                std::thread::spawn(move || {
                    for i in 1..=50 {
                        s.append(&format!("n{p}"), i, 0, b"x").unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(s.events_after(0, 1000).unwrap().len(), 200);
        for p in 0..4 {
            assert_eq!(s.acked(&format!("n{p}")).unwrap(), 50);
        }
    }

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("agora-hub-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Holds the database's write lock from another connection for `hold`, then commits: what another process does while it writes
    /// (the front door or the terminal client making a sign-in code, `agora hub enroll`).
    pub(crate) fn hold_write_lock(
        path: &Path,
        hold: std::time::Duration,
    ) -> std::thread::JoinHandle<()> {
        let other = Connection::open(path).unwrap();
        other
            .execute_batch(
                "BEGIN IMMEDIATE; INSERT OR REPLACE INTO settings (key, value) VALUES ('other', x'00');",
            )
            .unwrap();
        std::thread::spawn(move || {
            std::thread::sleep(hold);
            other.execute_batch("COMMIT;").unwrap();
        })
    }

    /// #935: a node's event arriving while another process writes to the database waits for it. The append read before it wrote, in
    /// a deferred transaction, and SQLite does not wait for the write lock in a transaction that already reads: it answered "database
    /// is locked" at once, and the hub dropped the node's connection without a close (the node saw "peer closed connection without
    /// sending TLS close_notify"). The hub's own events likewise.
    #[test]
    fn writes_wait_for_another_process_writing() {
        let d = tmpdir("busy");
        let path = d.join("hub.db");
        let s = SqliteStore::open(&path).unwrap();
        s.append("n", 1, 0, b"a").unwrap();
        let hold = std::time::Duration::from_millis(300);
        let other = hold_write_lock(&path, hold);
        let r = s.append("n", 2, 0, b"b");
        other.join().unwrap();
        assert!(matches!(r, Ok(Append::Applied { .. })), "{r:?}");
        let other = hold_write_lock(&path, hold);
        let r = s.append_batch_with_setting("hub", 1, &[(0, b"h".to_vec())], Some(("k", b"v")));
        other.join().unwrap();
        assert_eq!(r.map(|p| p.len()).ok(), Some(1));
        assert_eq!(s.acked("n").unwrap(), 2);
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn backup_is_consistent_and_restore_lets_nodes_replay_the_gap() {
        let d = tmpdir("backup");
        let live = d.join("hub.db");
        let s = SqliteStore::open(&live).unwrap();
        for i in 1..=3 {
            s.append("n", i, 0, &[i as u8]).unwrap();
        }
        s.set_setting("k", b"v").unwrap();
        let backup = d.join("backup.db");
        s.backup_to(&backup).unwrap();
        // Writes after the backup are lost by restoring it, until the node replays them.
        for i in 4..=5 {
            s.append("n", i, 0, &[i as u8]).unwrap();
        }
        assert!(s.backup_to(&backup).is_err(), "must not overwrite a backup");
        drop(s);

        restore(&backup, &live).unwrap();
        assert!(d.join("hub.db.before-restore").exists());
        let s = SqliteStore::open(&live).unwrap();
        assert_eq!(s.acked("n").unwrap(), 3);
        assert_eq!(s.setting("k").unwrap().as_deref(), Some(&b"v"[..]));
        // The node still holds 4 and 5 and replays them from the hub's new ack.
        assert_eq!(
            s.append("n", 4, 0, &[4]).unwrap(),
            Append::Applied { pos: 4 }
        );
        assert_eq!(
            s.append("n", 5, 0, &[5]).unwrap(),
            Append::Applied { pos: 5 }
        );
        assert_eq!(s.acked("n").unwrap(), 5);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restore_refuses_a_damaged_or_unrelated_file() {
        let d = tmpdir("badrestore");
        let live = d.join("hub.db");
        SqliteStore::open(&live)
            .unwrap()
            .set_setting("k", b"v")
            .unwrap();
        let junk = d.join("junk.db");
        std::fs::write(&junk, b"this is not a database at all, just some bytes").unwrap();
        assert!(restore(&junk, &live).is_err());
        let foreign = d.join("foreign.db");
        Connection::open(&foreign)
            .unwrap()
            .execute_batch("CREATE TABLE x (a); PRAGMA user_version = 99;")
            .unwrap();
        assert!(restore(&foreign, &live).is_err());
        // The live database is untouched.
        let s = SqliteStore::open(&live).unwrap();
        assert_eq!(s.setting("k").unwrap().as_deref(), Some(&b"v"[..]));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A hub made by the last release before team mode (schema 4) keeps working after an upgrade: its events and its owner's
    /// session survive, and what the new schemas add starts empty (ADR-0049).
    #[test]
    fn a_database_from_before_team_mode_upgrades_in_place() {
        let d = tmpdir("upgrade");
        let path = d.join("hub.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE events (pos INTEGER PRIMARY KEY AUTOINCREMENT, producer TEXT NOT NULL, seq INTEGER NOT NULL, at_ms INTEGER NOT NULL, payload BLOB NOT NULL, UNIQUE (producer, seq));
                 CREATE TABLE producers (producer TEXT PRIMARY KEY, last_seq INTEGER NOT NULL);
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value BLOB NOT NULL);
                 CREATE TABLE enrollment_codes (code_hash BLOB PRIMARY KEY, expires_ms INTEGER NOT NULL);
                 CREATE TABLE devices (serial TEXT PRIMARY KEY, node_id TEXT NOT NULL, enrolled_ms INTEGER NOT NULL, revoked_ms INTEGER);
                 CREATE TABLE signin_codes (code_hash BLOB PRIMARY KEY, expires_ms INTEGER NOT NULL);
                 CREATE TABLE sessions (token_hash BLOB PRIMARY KEY, created_ms INTEGER NOT NULL, expires_ms INTEGER NOT NULL);
                 CREATE TABLE passkeys (id TEXT PRIMARY KEY, public_key BLOB NOT NULL, sign_count INTEGER NOT NULL, created_ms INTEGER NOT NULL);
                 INSERT INTO events (producer, seq, at_ms, payload) VALUES ('n1', 1, 5, x'0a');
                 INSERT INTO settings (key, value) VALUES ('k', x'76');
                 INSERT INTO sessions (token_hash, created_ms, expires_ms) VALUES (x'aa', 1, 9999999999999);
                 PRAGMA user_version = 4;",
            )
            .unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        let conn = s.lock().unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1);
        // The old session names no person: the owner's, as before.
        let user: Option<String> = conn
            .query_row("SELECT user_id FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert!(user.is_none());
        // An old passkey names no person: it is the owner's, as before.
        let pk: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM passkeys WHERE user_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pk, 0);
        for table in ["users", "audit", "grants", "resyncs"] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{table}");
        }
        drop(conn);
        assert_eq!(s.setting("k").unwrap().as_deref(), Some(&b"v"[..]));
        // And reopening an upgraded database changes nothing.
        drop(s);
        assert!(SqliteStore::open(&path).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ADR-0054: after `SnapshotRequired` the producer's next event above what the hub holds becomes its new base, in one
    /// transaction with the record of the skipped numbers. Never backwards, and the next number is an ordinary append.
    #[test]
    fn a_resync_stores_the_event_moves_the_numbering_and_records_the_skipped_range() {
        let s = store();
        for i in 1..=3 {
            s.append("n1", i, 0, &[i as u8]).unwrap();
        }
        // At or below what is held stays a duplicate, even as a resync.
        assert_eq!(
            s.append_resync("n1", 2, 0, b"x", 99).unwrap(),
            Append::Duplicate
        );
        // The next number is an ordinary append: nothing was lost.
        assert_eq!(
            s.append_resync("n1", 4, 0, b"d", 99).unwrap(),
            Append::Applied { pos: 4 }
        );
        assert!(s.resyncs().unwrap().is_empty());
        assert_eq!(
            s.append_resync("n1", 10, 7, b"j", 99).unwrap(),
            Append::Resynced {
                pos: 5,
                from_seq: 5,
                to_seq: 9
            }
        );
        assert_eq!(s.acked("n1").unwrap(), 10);
        assert_eq!(s.resyncs().unwrap(), [("n1".to_string(), 5, 9, 99)]);
        let last = s.events_after(4, 10).unwrap();
        assert_eq!((last.len(), last[0].seq, last[0].at_ms), (1, 10, 7));
        // Ordinary appends follow the normal rules from the new base.
        assert_eq!(s.append("n1", 10, 0, b"j").unwrap(), Append::Duplicate);
        assert_eq!(
            s.append("n1", 12, 0, b"l").unwrap(),
            Append::Gap { expected: 11 }
        );
        // Another producer is untouched.
        assert_eq!(s.acked("n2").unwrap(), 0);
    }

    /// The resync record and the event are one transaction: an event that cannot be stored leaves no record and no jump.
    #[test]
    fn a_resync_that_fails_to_store_its_event_records_nothing() {
        let s = store();
        s.append("n1", 1, 0, b"a").unwrap();
        // An event time SQLite cannot hold is refused before anything is written.
        assert!(s.append_resync("n1", 5, u64::MAX, b"e", 9).is_err());
        assert_eq!(s.acked("n1").unwrap(), 1);
        assert!(s.resyncs().unwrap().is_empty());
        // A failure after the event row was written (the record's own time) rolls the event and the jump back too.
        assert!(s.append_resync("n1", 5, 0, b"e", u64::MAX).is_err());
        assert_eq!(s.acked("n1").unwrap(), 1);
        assert_eq!(s.events_after(0, 10).unwrap().len(), 1);
        assert!(s.resyncs().unwrap().is_empty());
    }

    /// Takes a current database back to schema 9: the audit trail as it was before it named sessions (L16).
    fn undo_audit_sessions(conn: &Connection) {
        conn.execute_batch(
            "DROP TRIGGER audit_only_the_outcome;
             ALTER TABLE audit DROP COLUMN session;
             CREATE TRIGGER audit_only_the_outcome BEFORE UPDATE ON audit
             WHEN OLD.outcome != 'sent' OR NEW.id != OLD.id OR NEW.at_ms != OLD.at_ms OR NEW.who != OLD.who
                  OR NEW.action != OLD.action OR NEW.node != OLD.node
             BEGIN SELECT RAISE(ABORT, 'the audit trail is append-only'); END;
             PRAGMA user_version = 9;",
        )
        .unwrap();
    }

    /// L16: a schema 9 database opens with its audit rows kept, each with no session, and the session is as fixed as the rest.
    #[test]
    fn an_audit_trail_from_before_sessions_upgrades_in_place() {
        let d = tmpdir("upgrade9");
        let path = d.join("hub.db");
        {
            let s = SqliteStore::open(&path).unwrap();
            let conn = s.lock().unwrap();
            undo_audit_sessions(&conn);
            conn.execute_batch(
                "INSERT INTO audit (at_ms, who, action, node, outcome) VALUES (1, 'owner', 'AgentStop', 'n1', 'sent');",
            )
            .unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        let rows = crate::audit::Audit::new(&s).list(0, None, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session, "");
        assert_eq!(rows[0].outcome, "sent");
        let conn = s.lock().unwrap();
        assert!(conn.execute("UPDATE audit SET session = 'x'", []).is_err());
        assert!(conn.execute("UPDATE audit SET who = 'x'", []).is_err());
        assert!(conn.execute("DELETE FROM audit", []).is_err());
        drop(conn);
        crate::audit::Audit::new(&s)
            .finish(rows[0].id, "ok")
            .unwrap();
        assert_eq!(
            crate::audit::Audit::new(&s).outcome(rows[0].id).as_deref(),
            Some("ok")
        );
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A hub database from before ADR-0054 (schema 8) opens, gains an empty `resyncs` table and keeps its events.
    #[test]
    fn a_database_from_before_resyncs_upgrades_in_place() {
        let d = tmpdir("upgrade8");
        let path = d.join("hub.db");
        {
            let s = SqliteStore::open(&path).unwrap();
            s.append("n1", 1, 5, b"a").unwrap();
            let conn = s.lock().unwrap();
            undo_audit_sessions(&conn);
            conn.execute_batch("DROP TABLE resyncs; PRAGMA user_version = 8;")
                .unwrap();
        }
        assert!(verify(&path).is_ok(), "an older database is a valid backup");
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(s.acked("n1").unwrap(), 1);
        assert!(s.resyncs().unwrap().is_empty());
        assert_eq!(
            s.append_resync("n1", 4, 0, b"d", 1).unwrap(),
            Append::Resynced {
                pos: 2,
                from_seq: 2,
                to_seq: 3
            }
        );
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ADR-0113 (schema 15), ADR-0114 (schema 16), ADR-0113 step 4 (schema 17), ADR-0114 step 4 (schema 18), then ADR-0120 (schema
    /// 19): a fresh database is at 19 with every table; one at 14 gains the theme catalog, the plugin tables, the settings table, the
    /// plugin catalog and the gallery's columns and screenshots, in that order; one at 15 keeps its catalog rows (their date and
    /// popularity empty), one at 16 its plugins too, and each gains what came after. Its events stay.
    /// ADR-0122: a schema-19 database gains the shared items' tables, empty, and keeps what it had.
    #[test]
    fn a_schema_19_database_gains_shared_items_and_task_review_tables() {
        let d = tmpdir("upgrade19");
        let path = d.join("hub.db");
        {
            let s = SqliteStore::open(&path).unwrap();
            s.append("n1", 1, 5, b"a").unwrap();
            s.lock()
                .unwrap()
                .execute_batch(
                    "DROP TABLE shared_items; DROP TABLE shared_item_refusals; \
                     DROP TABLE task_review_projects; PRAGMA user_version = 19;",
                )
                .unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        let conn = s.lock().unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        for t in [
            "shared_items",
            "shared_item_refusals",
            "task_review_projects",
        ] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{t}");
        }
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1);
    }

    #[test]
    fn databases_at_14_to_18_upgrade_to_the_gallery_columns() {
        let all = [
            "plugin_catalog",
            "plugin_catalog_screenshot",
            "plugin_grants",
            "plugins",
            "theme_catalog",
            "user_settings",
            "user_settings_tags",
        ];
        let tables = |s: &SqliteStore| -> Vec<String> {
            let conn = s.lock().unwrap();
            let mut st = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('theme_catalog', 'plugins', 'plugin_grants', 'user_settings', 'user_settings_tags', 'plugin_catalog', 'plugin_catalog_screenshot') ORDER BY name")
                .unwrap();
            let names = st.query_map([], |r| r.get(0)).unwrap();
            names.collect::<Result<_, _>>().unwrap()
        };
        let version = |s: &SqliteStore| -> i64 {
            s.lock()
                .unwrap()
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap()
        };
        let count = |s: &SqliteStore, table: &str| -> i64 {
            s.lock()
                .unwrap()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        };
        let fresh = store();
        assert_eq!(version(&fresh), SCHEMA_VERSION);
        assert_eq!(tables(&fresh), all);
        // What schema 19 added, taken out first (its columns are on tables an older database has).
        let gallery = "DROP TABLE shared_items; DROP TABLE shared_item_refusals; \
                       DROP TABLE plugin_catalog_screenshot; ALTER TABLE theme_catalog DROP COLUMN added; \
                       ALTER TABLE theme_catalog DROP COLUMN popularity; ALTER TABLE plugin_catalog DROP COLUMN added; \
                       ALTER TABLE plugin_catalog DROP COLUMN popularity;";
        for (from, undo) in [
            (18, "PRAGMA user_version = 18;"),
            (14, "DROP TABLE plugin_catalog; DROP TABLE user_settings; DROP TABLE user_settings_tags; DROP TABLE plugins; DROP TABLE plugin_grants; DROP TABLE theme_catalog; PRAGMA user_version = 14;"),
            (15, "DROP TABLE plugin_catalog; DROP TABLE user_settings; DROP TABLE user_settings_tags; DROP TABLE plugins; DROP TABLE plugin_grants; PRAGMA user_version = 15;"),
            (16, "DROP TABLE plugin_catalog; DROP TABLE user_settings; DROP TABLE user_settings_tags; PRAGMA user_version = 16;"),
            (17, "DROP TABLE plugin_catalog; PRAGMA user_version = 17;"),
        ] {
            let d = tmpdir(&format!("upgrade{from}"));
            let path = d.join("hub.db");
            {
                let s = SqliteStore::open(&path).unwrap();
                s.append("n1", 1, 5, b"a").unwrap();
                let conn = s.lock().unwrap();
                conn.execute_batch(gallery).unwrap();
                conn.execute_batch(undo).unwrap();
                if from >= 15 {
                    conn.execute(
                        "INSERT INTO theme_catalog (id, name, version, author, licence, sha256, data) VALUES ('t', 'T', 1, 'a', 'MIT', 'h', '{}')",
                        [],
                    )
                    .unwrap();
                }
                if from >= 16 {
                    conn.execute(
                        "INSERT INTO plugin_grants (user_id, plugin_id, version, seen, granted, set_ms) VALUES ('u', 'p', '1.0.0', '', '', 1)",
                        [],
                    )
                    .unwrap();
                }
            }
            assert!(verify(&path).is_ok(), "an older database is a valid backup");
            let s = SqliteStore::open(&path).unwrap();
            assert_eq!(version(&s), SCHEMA_VERSION, "from {from}");
            assert_eq!(tables(&s), all, "from {from}");
            assert_eq!(s.acked("n1").unwrap(), 1);
            assert_eq!(count(&s, "theme_catalog"), i64::from(from >= 15), "from {from}");
            assert_eq!(count(&s, "plugin_grants"), i64::from(from >= 16), "from {from}");
            assert_eq!(count(&s, "user_settings"), 0, "from {from}");
            assert_eq!(count(&s, "plugin_catalog"), 0, "from {from}");
            assert_eq!(count(&s, "plugin_catalog_screenshot"), 0, "from {from}");
            if from >= 15 {
                let kept: (String, i64) = s
                    .lock()
                    .unwrap()
                    .query_row("SELECT added, popularity FROM theme_catalog", [], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .unwrap();
                assert_eq!(kept, (String::new(), 0), "from {from}");
            }
            drop(s);
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    /// ADR-0085: a schema 12 database opens, keeps its enrolled devices and gains the `first_seen_ms` column, empty.
    #[test]
    fn a_database_from_before_certificate_renewal_upgrades_in_place() {
        let d = tmpdir("upgrade12");
        let path = d.join("hub.db");
        {
            let s = SqliteStore::open(&path).unwrap();
            let conn = s.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO devices (serial, node_id, enrolled_ms) VALUES ('ab', 'n1', 7);
                 ALTER TABLE devices DROP COLUMN first_seen_ms;
                 ALTER TABLE devices DROP COLUMN cert_pem;
                 ALTER TABLE devices DROP COLUMN csr_key_hash;
                 PRAGMA user_version = 12;",
            )
            .unwrap();
        }
        let s = SqliteStore::open(&path).unwrap();
        let conn = s.lock().unwrap();
        let (node, seen): (String, Option<i64>) = conn
            .query_row(
                "SELECT node_id, first_seen_ms FROM devices WHERE serial = 'ab'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((node.as_str(), seen), ("n1", None));
        drop(conn);
        drop(s);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn audit_attribution_migration_preserves_earlier_rows() {
        let d = tmpdir("audit-target-migration");
        let path = d.join("hub.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE devices (first_seen_ms INTEGER, cert_pem TEXT, csr_key_hash BLOB);
             CREATE TABLE audit (id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, who TEXT NOT NULL,
                 session TEXT NOT NULL, action TEXT NOT NULL, node TEXT NOT NULL, outcome TEXT NOT NULL);
             CREATE TRIGGER audit_only_the_outcome BEFORE UPDATE ON audit
                 WHEN OLD.outcome != 'sent' BEGIN SELECT RAISE(ABORT, 'append-only'); END;
             INSERT INTO audit VALUES (1, 10, 'owner', 'abc123', 'AgentStop', 'n1', 'ok');
             PRAGMA user_version = 13;",
        )
        .unwrap();
        drop(conn);

        let store = SqliteStore::open(&path).unwrap();
        let audit = crate::audit::Audit::new(&store);
        let rows = audit.list(0, None, 10).unwrap();
        let row = crate::audit::json_value(&rows[0]);
        assert_eq!(row["who"], "owner");
        assert_eq!(row["node"], "n1");
        assert_eq!(row["outcome"], "ok");
        for field in [
            "project_id",
            "agent_id",
            "requested_device_id",
            "signer_device_id",
        ] {
            assert_eq!(row[field], "");
        }
        drop(store);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An unreleased build of schema 13 left databases at `user_version = 13` without the renewal columns. Opening one adds them (and
    /// keeps the rows), so a renewal on it does not fail with "no such column".
    #[test]
    fn a_database_at_version_13_without_the_renewal_columns_gets_them_on_open() {
        let d = tmpdir("v13-missing-columns");
        let path = d.join("hub.db");
        {
            let s = SqliteStore::open(&path).unwrap();
            let conn = s.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO devices (serial, node_id, enrolled_ms) VALUES ('ab', 'n1', 7);
                 ALTER TABLE devices DROP COLUMN cert_pem;
                 ALTER TABLE devices DROP COLUMN csr_key_hash;
                 PRAGMA user_version = 13;",
            )
            .unwrap();
            let v: i64 = conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(v, 13);
        }
        let s = SqliteStore::open(&path).unwrap();
        let conn = s.lock().unwrap();
        let (node, pem, hash): (String, Option<String>, Option<Vec<u8>>) = conn
            .query_row(
                "SELECT node_id, cert_pem, csr_key_hash FROM devices WHERE serial = 'ab'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((node.as_str(), pem, hash), ("n1", None, None));
        drop(conn);
        // Opening again changes nothing.
        drop(s);
        assert!(SqliteStore::open(&path).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn append_after_i64_max_resync_returns_duplicate_without_overflow() {
        let store = SqliteStore::open_in_memory().unwrap();
        let resync = store
            .append_resync("n1", i64::MAX as u64, 1000, b"a", 1000)
            .unwrap();
        assert!(matches!(resync, Append::Resynced { .. }));
        let next = store.append("n1", 1, 1001, b"b").unwrap();
        assert_eq!(next, Append::Duplicate);
    }
}
