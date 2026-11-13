//! The audit trail (ADR-0049): who did what, where, and how it ended. Every routed command, sign-in, and change to the team is a row.
//! Terminal bytes and message bodies are never in it, only that something happened (ADR-0022). Rows are append-only, held by
//! triggers in the database itself (see `store.rs`), so no command and no bug in the hub can edit or delete one.

use rusqlite::{params, OptionalExtension};

use crate::store::{SqliteStore, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub at_ms: u64,
    pub who: String,
    /// The session that did it (L16): a short id of its token, `""` for the hub itself and for rows written before.
    pub session: String,
    pub action: String,
    pub node: String,
    pub outcome: String,
}

pub struct Audit<'a> {
    store: &'a SqliteStore,
}

fn db(e: rusqlite::Error) -> StoreError {
    StoreError(e.to_string())
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl<'a> Audit<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self { store }
    }

    /// Appends a row that no session made (the hub itself, the command line) and returns its id. `outcome` is `sent` for a command
    /// still on its way (see [`Self::finish`]), else what ended it.
    pub fn record(
        &self,
        at_ms: u64,
        who: &str,
        action: &str,
        node: &str,
        outcome: &str,
    ) -> Result<i64, StoreError> {
        self.record_in("", at_ms, who, action, node, outcome)
    }

    /// [`Self::record`] for a row a session made: `session` is its [`session_id`].
    pub fn record_in(
        &self,
        session: &str,
        at_ms: u64,
        who: &str,
        action: &str,
        node: &str,
        outcome: &str,
    ) -> Result<i64, StoreError> {
        let conn = self.store.lock()?;
        conn.execute(
            "INSERT INTO audit (at_ms, who, session, action, node, outcome) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![to_i64(at_ms), who, session, action, node, outcome],
        )
        .map_err(db)?;
        Ok(conn.last_insert_rowid())
    }

    /// Says how a `sent` command ended: `ok` or the error code. A row that is no longer `sent` is left as it is.
    pub fn finish(&self, id: i64, outcome: &str) -> Result<(), StoreError> {
        let conn = self.store.lock()?;
        conn.execute(
            "UPDATE audit SET outcome = ?2 WHERE id = ?1 AND outcome = 'sent'",
            params![id, outcome],
        )
        .map_err(db)?;
        Ok(())
    }

    /// The newest `limit` rows at or after `since_ms`, oldest first; `who` narrows it to one person.
    pub fn list(
        &self,
        since_ms: u64,
        who: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Entry>, StoreError> {
        let conn = self.store.lock()?;
        let mut st = conn
            .prepare(
                "SELECT id, at_ms, who, session, action, node, outcome FROM (
                     SELECT * FROM audit WHERE at_ms >= ?1 AND (?2 IS NULL OR who = ?2) ORDER BY id DESC LIMIT ?3
                 ) ORDER BY id",
            )
            .map_err(db)?;
        let rows = st
            .query_map(
                params![
                    to_i64(since_ms),
                    who,
                    i64::try_from(limit).unwrap_or(i64::MAX)
                ],
                |r| {
                    Ok(Entry {
                        id: r.get(0)?,
                        at_ms: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                        who: r.get(2)?,
                        session: r.get(3)?,
                        action: r.get(4)?,
                        node: r.get(5)?,
                        outcome: r.get(6)?,
                    })
                },
            )
            .map_err(db)?;
        rows.collect::<Result<_, _>>().map_err(db)
    }

    /// The outcome of one row (for tests and the page).
    pub fn outcome(&self, id: i64) -> Option<String> {
        let conn = self.store.lock().ok()?;
        conn.query_row("SELECT outcome FROM audit WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .ok()?
    }
}

/// One row as JSON.
/// A short id of a session for the audit trail: the first 6 hex digits of its token's hash. It tells two sessions apart on the page
/// and says nothing about the token (L16).
pub fn session_id(token_hash: &[u8]) -> String {
    token_hash
        .iter()
        .take(3)
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn json_value(e: &Entry) -> serde_json::Value {
    serde_json::json!({
        "id": e.id, "at_ms": e.at_ms, "who": e.who, "session": e.session, "action": e.action, "node": e.node, "outcome": e.outcome,
    })
}

/// One row as a JSON line (`agora hub audit export`).
pub fn json_line(e: &Entry) -> String {
    json_value(e).to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_appended_finished_once_and_never_edited_or_deleted() {
        let store = SqliteStore::open_in_memory().unwrap();
        let audit = Audit::new(&store);
        let a = audit
            .record(10, "owner", "AgentStop", "n1", "sent")
            .unwrap();
        let b = audit.record(11, "eda", "sign_in", "", "ok").unwrap();
        audit.finish(a, "ok").unwrap();
        audit.finish(a, "node.timeout").unwrap(); // already finished: unchanged
        assert_eq!(audit.outcome(a).as_deref(), Some("ok"));
        assert_eq!(audit.list(0, None, 10).unwrap().len(), 2);
        assert_eq!(audit.list(0, Some("eda"), 10).unwrap()[0].id, b);
        assert_eq!(audit.list(11, None, 10).unwrap().len(), 1);
        assert_eq!(
            audit.list(0, None, 1).unwrap()[0].id,
            b,
            "the newest rows win the limit"
        );
        let conn = store.lock().unwrap();
        assert!(conn.execute("DELETE FROM audit", []).is_err());
        assert!(conn.execute("UPDATE audit SET who = 'x'", []).is_err());
        assert!(conn
            .execute("UPDATE audit SET outcome = 'x' WHERE id = 1", [])
            .is_err());
    }
}
