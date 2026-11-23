//! The audit trail (ADR-0049): who did what, where, and how it ended. Every routed command, sign-in, and change to the team is a row.
//! Terminal bytes and message bodies are never in it, only that something happened (ADR-0022). Rows are append-only, held by
//! triggers in the database itself (see `store.rs`), so no command and no bug in the hub can edit or delete one.

use agora_protocol::v1::{command_request::Command as C, command_response, CommandResponse};
use rusqlite::{params, OptionalExtension};

use crate::projection::{qualified, split_key, View};
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
    pub project_id: String,
    pub agent_id: String,
    pub requested_device_id: String,
    pub signer_verified: bool,
}

#[derive(Default)]
pub(crate) struct Target<'a> {
    pub node: &'a str,
    project_id: String,
    agent_id: String,
    requested_device_id: String,
}

enum Resource<'a> {
    Machine,
    Project(&'a str),
    Agent(&'a str),
    Approval(&'a str),
    Terminal(&'a str),
    Workspace(&'a str),
    Worktree(&'a str),
    Checkout(&'a str),
}

impl<'a> Target<'a> {
    /// Uses only identifiers documented by the command and the hub's current node-qualified projection.
    pub(crate) fn for_command(node: &'a str, command: Option<&C>, view: &View) -> Self {
        use Resource::*;
        let mut target = Self {
            node,
            ..Self::default()
        };
        let Some(cmd) = command else { return target };
        let resource = match cmd {
            C::AgentStart(c) => Project(&c.project_id),
            C::AgentLaunch(c) => Project(&c.project_id),
            C::AgentResume(c) => Agent(&c.agent_id),
            C::AgentStop(c) => Agent(&c.agent_id),
            C::AgentPrompt(c) => Agent(&c.agent_id),
            C::AgentSetMode(c) => Agent(&c.agent_id),
            C::AgentSetModel(c) => Agent(&c.agent_id),
            C::AgentSetConfig(c) => Agent(&c.agent_id),
            C::AgentCancelTurn(c) => Agent(&c.agent_id),
            C::ChatSubscribe(c) => Agent(&c.agent_id),
            C::TerminalInput(c) => Terminal(&c.terminal_id),
            C::TerminalResize(c) => Terminal(&c.terminal_id),
            C::TerminalSubscribe(c) => Terminal(&c.terminal_id),
            C::WorkspaceQuery(c) => Workspace(&c.workspace_id),
            C::FilesList(c) => Worktree(&c.worktree_id),
            C::FilesRead(c) => Worktree(&c.worktree_id),
            C::FilesWrite(c) => Worktree(&c.worktree_id),
            C::FilesWatch(c) => Worktree(&c.worktree_id),
            C::GitDiff(c) => Worktree(&c.worktree_id),
            C::GitStatus(c) => Worktree(&c.worktree_id),
            C::GitLog(c) => Worktree(&c.worktree_id),
            C::WorktreeRemove(c) => Worktree(&c.worktree_id),
            C::WorktreeCreate(c) => Checkout(&c.checkout_id),
            C::ApprovalResolve(c) => Approval(&c.approval_id),
            C::TaskUpdate(c) => Project(&c.project_id),
            C::TaskDispatch(c) => {
                target.project_id = bounded_id(&c.project_id);
                Agent(&c.agent_id)
            }
            C::GrantCreate(c) => match c.scope.split_once(':') {
                Some(("agent", id)) => Agent(id),
                Some(("project", id)) => Project(id),
                _ => Machine,
            },
            _ => Machine,
        };
        let q = |id: &str| qualified(node, id);
        // Approval and terminal records carry the qualified agent id. Any other node prefix is outside this command's scope.
        let local_id = |key: &str| match split_key(key) {
            Some((producer, id)) if producer == node => bounded_id(id),
            _ => String::new(),
        };
        let checkout_project = |key: &str| view.checkouts.get(key).map(|c| c.1.as_str());
        let project = match resource {
            Machine => None,
            Project(id) => Some(id),
            Agent(id) => {
                target.agent_id = bounded_id(id);
                None
            }
            Approval(id) => {
                if let Some(approval) = view.approvals.get(&q(id)) {
                    target.agent_id = local_id(&approval.agent_id);
                }
                None
            }
            Terminal(id) => {
                if let Some(terminal) = view.terminals.get(&q(id)) {
                    target.agent_id = local_id(&terminal.agent_id);
                }
                None
            }
            Workspace(id) => view.workspaces.get(&q(id)).map(|w| w.project_id.as_str()),
            Worktree(id) => view
                .worktrees
                .get(&q(id))
                .and_then(|w| checkout_project(&w.checkout_id)),
            Checkout(id) => checkout_project(&q(id)),
        };
        if let Some(id) = project {
            target.project_id = bounded_id(id);
        } else if target.project_id.is_empty() && !target.agent_id.is_empty() {
            if let Some(agent) = view.agents.get(&q(&target.agent_id)) {
                target.project_id = bounded_id(&agent.project_id);
            }
        }
        let device = match cmd {
            C::ApprovalResolve(c) => Some(c.signer_device_id.as_str()),
            C::GrantCreate(c) => Some(c.signer_device_id.as_str()),
            C::AgentLaunch(c) => c.authorization.as_ref().map(|a| a.device_id.as_str()),
            C::AgentResume(c) => c.authorization.as_ref().map(|a| a.device_id.as_str()),
            C::DirectoryList(c) => c.authorization.as_ref().map(|a| a.device_id.as_str()),
            C::DirectoryInspect(c) => c.authorization.as_ref().map(|a| a.device_id.as_str()),
            _ => None,
        };
        target.requested_device_id = bounded_id(device.unwrap_or_default());
        target
    }
}

fn bounded_id(id: &str) -> String {
    const MAX_ID_BYTES: usize = 256;
    // A clipped id could look like a different, valid id.
    if id.len() > MAX_ID_BYTES || id.chars().any(char::is_control) {
        return String::new();
    }
    id.to_string()
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
        self.record_target(
            session,
            at_ms,
            who,
            action,
            outcome,
            &Target {
                node,
                ..Target::default()
            },
        )
    }

    pub(crate) fn record_target(
        &self,
        session: &str,
        at_ms: u64,
        who: &str,
        action: &str,
        outcome: &str,
        target: &Target<'_>,
    ) -> Result<i64, StoreError> {
        let conn = self.store.lock()?;
        conn.execute(
            "INSERT INTO audit (at_ms, who, session, action, node, outcome, project_id, agent_id, requested_device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![to_i64(at_ms), who, session, action, target.node, outcome,
                    target.project_id, target.agent_id, target.requested_device_id],
        )
        .map_err(db)?;
        Ok(conn.last_insert_rowid())
    }

    /// Says how a `sent` command ended: `ok` or the error code. A row that is no longer `sent` is left as it is.
    pub fn finish(&self, id: i64, outcome: &str) -> Result<(), StoreError> {
        self.settle(id, outcome, false)
    }

    pub(crate) fn finish_response(
        &self,
        id: i64,
        response: &CommandResponse,
        fresh: bool,
    ) -> Result<(), StoreError> {
        match &response.result {
            Some(command_response::Result::Ok(_)) => self.settle(id, "ok", fresh),
            Some(command_response::Result::Err(info)) => self.finish(id, &info.code),
            None => self.finish(id, "ok"),
        }
    }

    fn settle(&self, id: i64, outcome: &str, accepted: bool) -> Result<(), StoreError> {
        let conn = self.store.lock()?;
        conn.execute(
            "UPDATE audit SET outcome = ?2, signer_verified = (?3 AND requested_device_id != '')
             WHERE id = ?1 AND outcome = 'sent'",
            params![id, outcome, accepted],
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
                "SELECT id, at_ms, who, session, action, node, outcome,
                        project_id, agent_id, requested_device_id, signer_verified FROM (
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
                        project_id: r.get(7)?,
                        agent_id: r.get(8)?,
                        requested_device_id: r.get(9)?,
                        signer_verified: r.get(10)?,
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
        "project_id": e.project_id, "agent_id": e.agent_id, "requested_device_id": e.requested_device_id,
        "signer_device_id": if e.signer_verified { e.requested_device_id.as_str() } else { "" },
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
        assert!(conn
            .execute("UPDATE audit SET project_id = 'other'", [])
            .is_err());
        assert!(conn
            .execute("UPDATE audit SET signer_verified = 1", [])
            .is_err());
        assert!(conn.execute("UPDATE audit SET who = 'x'", []).is_err());
        assert!(conn
            .execute("UPDATE audit SET outcome = 'x' WHERE id = 1", [])
            .is_err());
    }
}
