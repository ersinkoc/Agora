//! Team mode (ADR-0049): the people who use a hub and what each may do. A hub with no users is one owner's, as before; a session
//! with no user is the owner's. Roles are the hub's own limit on its clients; a node still trusts device keys, not roles (ADR-0020).

use rusqlite::{params, OptionalExtension};

use crate::store::{SqliteStore, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Reads only: no commands, no input, no approvals.
    Viewer,
    /// Works in projects: starts and stops agents, types, approves.
    Member,
    /// Everything, and manages users.
    Owner,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Self::Owner),
            "member" => Some(Self::Member),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
            Self::Viewer => "viewer",
        }
    }
}

/// Whose connection this is and what they may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub role: Role,
    /// The person's id; `owner` for a session that names nobody.
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: Role,
}

pub struct Users<'a> {
    store: &'a SqliteStore,
}

fn db(e: rusqlite::Error) -> StoreError {
    StoreError(e.to_string())
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

impl<'a> Users<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self { store }
    }

    /// Adds a person. The name is what people see; the id (a slug of it, made unique) is what sessions refer to.
    pub fn add(&self, name: &str, role: Role, now_ms: u64) -> Result<User, StoreError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(StoreError("a name is 1 to 64 characters".into()));
        }
        let base: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let base = base.trim_matches('-').to_string();
        let base = if base.is_empty() {
            "user".to_string()
        } else {
            base
        };
        let conn = self.store.lock()?;
        let mut id = base.clone();
        let mut n = 1;
        while conn
            .query_row("SELECT 1 FROM users WHERE id = ?1", [&id], |_| Ok(()))
            .optional()
            .map_err(db)?
            .is_some()
        {
            n += 1;
            id = format!("{base}-{n}");
        }
        conn.execute(
            "INSERT INTO users (id, name, role, created_ms) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, role.as_str(), to_i64(now_ms)],
        )
        .map_err(db)?;
        drop(conn);
        let _ = crate::audit::Audit::new(self.store).record(
            now_ms,
            "owner",
            "user.add",
            &id,
            role.as_str(),
        );
        Ok(User {
            id,
            name: name.into(),
            role,
        })
    }

    pub fn list(&self) -> Result<Vec<User>, StoreError> {
        let conn = self.store.lock()?;
        let mut st = conn
            .prepare("SELECT id, name, role FROM users ORDER BY created_ms, id")
            .map_err(db)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(db)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, role) = row.map_err(db)?;
            if let Some(role) = Role::parse(&role) {
                out.push(User { id, name, role });
            }
        }
        Ok(out)
    }

    /// Changes a role. Ends the user's sessions only when the role drops, so a demotion cannot wait for a session to expire.
    pub fn set_role(&self, id: &str, role: Role) -> Result<bool, StoreError> {
        let conn = self.store.lock()?;
        let n = conn
            .execute(
                "UPDATE users SET role = ?2 WHERE id = ?1",
                params![id, role.as_str()],
            )
            .map_err(db)?;
        drop(conn);
        if n > 0 {
            let _ = crate::audit::Audit::new(self.store).record(
                crate::server::now_ms(),
                "owner",
                "user.role",
                id,
                role.as_str(),
            );
        }
        Ok(n > 0)
    }

    /// Removes a person and every session and unused sign-in code of theirs.
    pub fn remove(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.store.lock()?;
        conn.execute("DELETE FROM sessions WHERE user_id = ?1", [id])
            .map_err(db)?;
        conn.execute("DELETE FROM signin_codes WHERE user_id = ?1", [id])
            .map_err(db)?;
        let n = conn
            .execute("DELETE FROM users WHERE id = ?1", [id])
            .map_err(db)?;
        drop(conn);
        if n > 0 {
            let _ = crate::audit::Audit::new(self.store).record(
                crate::server::now_ms(),
                "owner",
                "user.remove",
                id,
                "ok",
            );
        }
        Ok(n > 0)
    }

    /// Who holds this session token and what they may do: the owner when the session names no person, nothing when the person is gone.
    pub fn access_of_session(&self, token_hash: &[u8]) -> Option<Access> {
        let conn = self.store.lock().ok()?;
        let user: Option<Option<String>> = conn
            .query_row(
                "SELECT user_id FROM sessions WHERE token_hash = ?1",
                [token_hash],
                |r| r.get(0),
            )
            .optional()
            .ok()?;
        match user? {
            None => Some(Access {
                role: Role::Owner,
                user: "owner".into(),
            }),
            Some(id) => {
                let role: String = conn
                    .query_row("SELECT role FROM users WHERE id = ?1", [&id], |r| r.get(0))
                    .optional()
                    .ok()??;
                Some(Access {
                    role: Role::parse(&role)?,
                    user: id,
                })
            }
        }
    }
}

/// What a viewer may still do: everything that only reads. The command names are the schema's, so a new command is refused to
/// viewers until it is listed here.
pub fn viewer_may(command: &agora_protocol::v1::command_request::Command) -> bool {
    use agora_protocol::v1::command_request::Command as C;
    matches!(
        command,
        C::ProjectList(_)
            | C::AgentList(_)
            | C::AgentKinds(_)
            | C::UsageReport(_)
            | C::FilesList(_)
            | C::FilesRead(_)
            | C::FilesWatch(_)
            | C::GitDiff(_)
            | C::GitStatus(_)
            | C::GitLog(_)
            | C::NodePolicyGet(_)
            | C::GrantList(_)
            | C::TerminalSubscribe(_)
            | C::ChatSubscribe(_)
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn people_are_added_listed_demoted_and_removed_with_their_sessions() {
        let store = SqliteStore::open_in_memory().unwrap();
        let users = Users::new(&store);
        let a = users.add("Ayşe K.", Role::Member, 1).unwrap();
        let b = users.add("Ayşe K.", Role::Viewer, 2).unwrap();
        assert_ne!(a.id, b.id);
        assert!(users.add("  ", Role::Member, 3).is_err());
        assert_eq!(users.list().unwrap().len(), 2);
        assert!(users.set_role(&b.id, Role::Member).unwrap());
        assert!(!users.set_role("nobody", Role::Member).unwrap());
        assert!(users.remove(&a.id).unwrap() && !users.remove(&a.id).unwrap());
        assert_eq!(users.list().unwrap().len(), 1);
    }

    #[test]
    fn a_viewer_reads_and_a_member_or_owner_does_anything_the_node_allows() {
        use agora_protocol::v1::{self, command_request::Command as C};
        assert!(viewer_may(&C::ProjectList(v1::ProjectList::default())));
        assert!(!viewer_may(&C::AgentStop(v1::AgentStop::default())));
        assert!(!viewer_may(&C::TerminalInput(v1::TerminalInput::default())));
        assert!(!viewer_may(&C::ApprovalResolve(
            v1::ApprovalResolve::default()
        )));
        assert!(Role::Owner > Role::Member && Role::Member > Role::Viewer);
    }
}
