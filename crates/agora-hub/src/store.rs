//! Hub storage (ADR-0014): an append-only event log with exactly-once application per `(producer, seq)`,
//! plus key-value settings. Embedded SQLite in WAL mode. The trait avoids SQLite-only concepts so a
//! Postgres backend can implement it later (Phase 7).
//!
//! Protocol rule (docs/specs/protocol.md §3): a producer numbers its events 1, 2, 3, …; the hub applies each
//! at most once and acknowledges the highest contiguous number.

use std::fmt;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

/// Outcome of [`EventStore::append`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Append {
    /// Stored; `pos` is its position in the global log (1, 2, 3, …).
    Applied { pos: u64 },
    /// Already applied earlier; nothing changed. The producer may drop it.
    Duplicate,
    /// Not the next event from this producer; `expected` is the one the hub is waiting for.
    Gap { expected: u64 },
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
    /// Highest contiguous `seq` applied for the producer (0 when none): the acknowledgement to send back.
    fn acked(&self, producer: &str) -> Result<u64, StoreError>;
    /// Events with a global position greater than `after`, oldest first, at most `limit`.
    fn events_after(&self, after: u64, limit: usize) -> Result<Vec<StoredEvent>, StoreError>;
    fn setting(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn set_setting(&self, key: &str, value: &[u8]) -> Result<(), StoreError>;
}

const SCHEMA_VERSION: i64 = 2;

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
        Ok(Self {
            conn: Mutex::new(conn),
        })
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

impl EventStore for SqliteStore {
    fn append(
        &self,
        producer: &str,
        seq: u64,
        at_ms: u64,
        payload: &[u8],
    ) -> Result<Append, StoreError> {
        let (seq_i, at_i) = (to_i64(seq)?, to_i64(at_ms)?);
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let last: i64 = tx
            .query_row(
                "SELECT last_seq FROM producers WHERE producer = ?1",
                [producer],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let expected = last + 1;
        if seq_i <= last {
            return Ok(Append::Duplicate);
        }
        if seq_i != expected {
            return Ok(Append::Gap {
                expected: to_u64(expected),
            });
        }
        tx.execute(
            "INSERT INTO events (producer, seq, at_ms, payload) VALUES (?1, ?2, ?3, ?4)",
            params![producer, seq_i, at_i, payload],
        )?;
        let pos = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO producers (producer, last_seq) VALUES (?1, ?2)
             ON CONFLICT(producer) DO UPDATE SET last_seq = excluded.last_seq",
            params![producer, seq_i],
        )?;
        tx.commit()?;
        Ok(Append::Applied { pos: to_u64(pos) })
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
}
