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
    /// The projects (`node/project`) this person may see and work in; `None` means all of them (the owner).
    pub projects: Option<std::collections::BTreeSet<String>>,
    /// The session's short id for the audit trail ([`crate::audit::session_id`]); `""` when there is no session (L16).
    pub session: String,
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
        // The ids the hub itself writes (`owner` for the owner's sessions and audit rows, `hub` for the shared board) are never a person's.
        let reserved = |id: &str| matches!(id, "owner" | "hub");
        // The devices a removed person registered stay listed under their id (the owner may still revoke them, ADR-0061), so that id
        // is never given to someone else: the new person would get them as theirs, and machines would be offered them (audit F910).
        let mut id = base.clone();
        let mut n = 1;
        while reserved(&id)
            || conn
                .query_row(
                    "SELECT 1 FROM users WHERE id = ?1
                     UNION ALL SELECT 1 FROM user_devices WHERE user_id = ?1 LIMIT 1",
                    [&id],
                    |_| Ok(()),
                )
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

    /// Whether the hub has any person at all, whatever the role: a row whose role this build cannot read counts too (it is left out
    /// of [`list`](Self::list)), so the hub is never taken for a solo one by mistake.
    pub fn any(&self) -> Result<bool, StoreError> {
        let conn = self.store.lock()?;
        conn.query_row("SELECT 1 FROM users LIMIT 1", [], |_| Ok(()))
            .optional()
            .map(|r| r.is_some())
            .map_err(db)
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

    /// Changes a role. Sessions are kept: the hub asks for the role on every command, and ends an open connection whose role or
    /// grants changed when it next checks (once a second, `server::serve_client_as`), so a demotion cannot wait for a session to expire.
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
        conn.execute("DELETE FROM grants WHERE user_id = ?1", [id])
            .map_err(db)?;
        conn.execute("DELETE FROM passkeys WHERE user_id = ?1", [id])
            .map_err(db)?;
        // ADR-0114: what this person allowed plugins to see goes with them.
        conn.execute("DELETE FROM plugin_grants WHERE user_id = ?1", [id])
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
                projects: None,
                session: crate::audit::session_id(token_hash),
            }),
            Some(id) => {
                let role: String = conn
                    .query_row("SELECT role FROM users WHERE id = ?1", [&id], |r| r.get(0))
                    .optional()
                    .ok()??;
                let role = Role::parse(&role)?;
                let projects = if role == Role::Owner {
                    None
                } else {
                    let mut st = conn
                        .prepare("SELECT node, project FROM grants WHERE user_id = ?1")
                        .ok()?;
                    let rows = st
                        .query_map([&id], |r| {
                            Ok(grant_key(&r.get::<_, String>(0)?, &r.get::<_, String>(1)?))
                        })
                        .ok()?;
                    Some(rows.filter_map(Result::ok).collect())
                };
                Some(Access {
                    role,
                    user: id,
                    projects,
                    session: crate::audit::session_id(token_hash),
                })
            }
        }
    }

    /// Lets a person see and work in a project of a machine. ADR-0115: the project is stored folded, so a later revoke in either case
    /// of a GitHub, GitLab, Bitbucket or Codeberg path finds it.
    pub fn grant(&self, id: &str, node: &str, project: &str) -> Result<bool, StoreError> {
        let project = agora_core::fold_project_id(project);
        let project = project.as_ref();
        let conn = self.store.lock()?;
        let known = conn
            .query_row("SELECT 1 FROM users WHERE id = ?1", [id], |_| Ok(()))
            .optional()
            .map_err(db)?
            .is_some();
        if !known {
            return Ok(false);
        }
        conn.execute(
            "INSERT OR IGNORE INTO grants (user_id, node, project) VALUES (?1, ?2, ?3)",
            params![id, node, project],
        )
        .map_err(db)?;
        drop(conn);
        let _ = crate::audit::Audit::new(self.store).record(
            crate::server::now_ms(),
            "owner",
            "user.grant",
            &format!("{id} {node}/{project}"),
            "ok",
        );
        Ok(true)
    }

    /// Takes a project of a machine back from a person. ADR-0115: every row that names it, in whatever case an older build stored a
    /// GitHub, GitLab, Bitbucket or Codeberg path, so no row spelled otherwise keeps the access.
    pub fn revoke(&self, id: &str, node: &str, project: &str) -> Result<bool, StoreError> {
        let conn = self.store.lock()?;
        let stored: Vec<String> = {
            let mut st = conn
                .prepare("SELECT project FROM grants WHERE user_id = ?1 AND node = ?2")
                .map_err(db)?;
            let rows = st
                .query_map(params![id, node], |r| r.get::<_, String>(0))
                .map_err(db)?;
            rows.collect::<Result<_, _>>().map_err(db)?
        };
        let mut n = 0;
        for p in stored
            .iter()
            .filter(|p| agora_core::same_project(p, project))
        {
            n += conn
                .execute(
                    "DELETE FROM grants WHERE user_id = ?1 AND node = ?2 AND project = ?3",
                    params![id, node, p],
                )
                .map_err(db)?;
        }
        drop(conn);
        if n > 0 {
            let _ = crate::audit::Audit::new(self.store).record(
                crate::server::now_ms(),
                "owner",
                "user.revoke",
                &format!("{id} {node}/{project}"),
                "ok",
            );
        }
        Ok(n > 0)
    }

    pub fn grants_of(&self, id: &str) -> Result<Vec<String>, StoreError> {
        let conn = self.store.lock()?;
        let mut st = conn
            .prepare("SELECT node, project FROM grants WHERE user_id = ?1 ORDER BY node, project")
            .map_err(db)?;
        let rows = st
            .query_map([id], |r| {
                Ok(grant_key(&r.get::<_, String>(0)?, &r.get::<_, String>(1)?))
            })
            .map_err(db)?;
        // ADR-0115: two rows an older build stored in two cases of one project are one grant.
        let keys: std::collections::BTreeSet<String> =
            rows.collect::<Result<_, _>>().map_err(db)?;
        Ok(keys.into_iter().collect())
    }
}

/// A grant as `node/project`, the project folded (ADR-0115): a row an older build stored in another case of a GitHub, GitLab,
/// Bitbucket or Codeberg path names the project the machines now report.
fn grant_key(node: &str, project: &str) -> String {
    format!("{node}/{}", agora_core::fold_project_id(project))
}

/// ADR-0112: the hub setting with which the owner lets agents see every machine and agent of a hub that has people besides the
/// owner (`agora hub fleet on|off`). Off when missing. A solo hub (no person in `users`) answers without it.
pub const FLEET_SETTING: &str = "fleet_status";
const FLEET_ON: &[u8] = b"on";

/// Whether the owner turned the fleet view on ([`FLEET_SETTING`]).
pub fn fleet_switch(store: &SqliteStore) -> Result<bool, StoreError> {
    use crate::store::EventStore;
    Ok(store.setting(FLEET_SETTING)?.as_deref() == Some(FLEET_ON))
}

/// Turns the fleet view on or off for a hub with people besides the owner, and records it in the audit trail as the owner's.
pub fn set_fleet_switch(store: &SqliteStore, on: bool, now_ms: u64) -> Result<(), StoreError> {
    use crate::store::EventStore;
    store.set_setting(FLEET_SETTING, if on { FLEET_ON } else { b"off" })?;
    let _ = crate::audit::Audit::new(store).record(
        now_ms,
        "owner",
        if on { "fleet_on" } else { "fleet_off" },
        "cli",
        "ok",
    );
    Ok(())
}

/// ADR-0112: whether agents may see every machine and agent of this hub (`BoardCall{tool: "fleet_status"}`): in solo mode (no
/// person in `users` at all, ADR-0049: the hub is the owner's alone), or when the owner turned [`FLEET_SETTING`] on. Any person
/// added, whatever the role (one this build cannot read included), makes it a team hub. A store that cannot be read refuses.
pub fn fleet_allowed(store: &SqliteStore) -> bool {
    match Users::new(store).any() {
        Ok(false) => true,
        Ok(true) => fleet_switch(store).unwrap_or(false),
        Err(_) => false,
    }
}

/// What a viewer may still do: everything that only reads. The command names are the schema's, so a new command is refused to
/// viewers until it is listed here.
pub fn viewer_may(command: &agora_protocol::v1::command_request::Command) -> bool {
    use agora_protocol::v1::command_request::Command as C;
    matches!(
        command,
        C::WorkspaceQuery(_)
            | C::ProjectList(_)
            | C::AgentList(_)
            | C::AgentKinds(_)
            | C::UsageReport(_)
            | C::ThemeCatalogList(_)
            | C::ThemeCatalogGet(_)
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

    /// ADR-0112, F3 of the #1205 review: a person whose role this build cannot read still makes the hub a team hub.
    #[test]
    fn a_person_with_an_unknown_role_ends_solo_mode() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert!(fleet_allowed(&store));
        store
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO users (id, name, role, created_ms) VALUES ('x', 'X', 'admin', 1)",
                [],
            )
            .unwrap();
        assert!(Users::new(&store).list().unwrap().is_empty());
        assert!(Users::new(&store).any().unwrap());
        assert!(!fleet_allowed(&store));
        set_fleet_switch(&store, true, 2).unwrap();
        assert!(fleet_allowed(&store));
    }

    #[test]
    fn people_are_added_listed_demoted_and_removed_with_their_sessions() {
        let store = SqliteStore::open_in_memory().unwrap();
        let users = Users::new(&store);
        let a = users.add("Ayşe K.", Role::Member, 1).unwrap();
        let b = users.add("Ayşe K.", Role::Viewer, 2).unwrap();
        assert_ne!(a.id, b.id);
        assert!(users.add("  ", Role::Member, 3).is_err());
        // The ids the hub writes itself are never a person's, so nobody can write audit rows that look like the owner's.
        let o = users.add("Owner", Role::Member, 4).unwrap();
        let h = users.add("hub", Role::Member, 5).unwrap();
        assert!(o.id != "owner" && h.id != "hub", "{} {}", o.id, h.id);
        assert_eq!(users.list().unwrap().len(), 4);
        assert!(users.set_role(&b.id, Role::Member).unwrap());
        assert!(!users.set_role("nobody", Role::Member).unwrap());
        assert!(users.remove(&a.id).unwrap() && !users.remove(&a.id).unwrap());
        assert_eq!(users.list().unwrap().len(), 3);
    }

    /// ADR-0115: a grant an older hub stored in a GitHub repository's own case is read folded, a new grant is stored folded, and a
    /// revoke in either case takes every row of that project, so no row keeps the access. Another project stays.
    #[test]
    fn a_grant_in_two_cases_of_one_github_project_is_one_grant_and_one_revoke() {
        let store = SqliteStore::open_in_memory().unwrap();
        let users = Users::new(&store);
        let a = users.add("Ayse", Role::Member, 1).unwrap();
        let root = "0123456789abcdef0123456789abcdef01234567";
        let (cased, folded) = (
            format!("github.com/O/R#{root}"),
            format!("github.com/o/r#{root}"),
        );
        let other = format!("git.example.org/O/R#{root}");
        store
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO grants (user_id, node, project) VALUES (?1, 'mac', ?2), (?1, 'mac', ?3)",
                params![a.id, cased, other],
            )
            .unwrap();
        assert!(users.grant(&a.id, "mac", &cased).unwrap());
        assert_eq!(
            users.grants_of(&a.id).unwrap(),
            [format!("mac/{other}"), format!("mac/{folded}")]
        );
        assert!(users.revoke(&a.id, "mac", &folded).unwrap());
        assert_eq!(users.grants_of(&a.id).unwrap(), [format!("mac/{other}")]);
        assert!(!users
            .revoke(&a.id, "mac", &format!("git.example.org/o/r#{root}"))
            .unwrap());
        assert!(users.revoke(&a.id, "mac", &other).unwrap());
        assert!(users.grants_of(&a.id).unwrap().is_empty());
    }

    /// Audit F910: the devices a removed person registered stay under their id, so someone added later under the same name gets an
    /// id of their own and never those devices (as theirs, or offered to a machine they work on).
    #[test]
    fn a_new_person_never_gets_the_devices_of_a_removed_person_of_the_same_name() {
        use crate::devices::{tests::Key, Devices};
        let store = SqliteStore::open_in_memory().unwrap();
        let (users, devices) = (Users::new(&store), Devices::new(&store));
        let ids = |v: Vec<agora_protocol::v1::DeviceInfo>| -> Vec<String> {
            v.into_iter().map(|d| d.device_id).collect()
        };
        let first = users.add("Eda", Role::Member, 1).unwrap();
        devices
            .register(&first.id, &Key::new().register("old-phone", "x", 1), 1)
            .unwrap();
        assert!(users.remove(&first.id).unwrap());
        let second = users.add("Eda", Role::Member, 2).unwrap();
        users.grant(&second.id, "laptop", "web").unwrap();
        assert_ne!(second.id, first.id);
        assert!(devices.list_for(&second.id).unwrap().is_empty());
        assert!(ids(devices.list_for_node("laptop").unwrap()).is_empty());
        assert_eq!(ids(devices.list_for(&first.id).unwrap()), ["old-phone"]);
        // A removed person without devices leaves nothing behind: the id may be given again.
        let lone = users.add("Ali", Role::Member, 3).unwrap();
        assert!(users.remove(&lone.id).unwrap());
        assert_eq!(users.add("Ali", Role::Member, 4).unwrap().id, lone.id);
    }

    #[test]
    fn a_viewer_reads_and_a_member_or_owner_does_anything_the_node_allows() {
        use agora_protocol::v1::{self, command_request::Command as C};
        assert!(viewer_may(&C::ProjectList(v1::ProjectList::default())));
        assert!(!viewer_may(&C::AgentStop(v1::AgentStop::default())));
        // ADR-0090: a viewer may neither change an agent's options nor cancel its turn.
        assert!(!viewer_may(&C::AgentSetConfig(
            v1::AgentSetConfig::default()
        )));
        assert!(!viewer_may(&C::AgentCancelTurn(
            v1::AgentCancelTurn::default()
        )));
        // ADR-0095: a viewer may not give a task to an agent.
        assert!(!viewer_may(&C::TaskDispatch(v1::TaskDispatch::default())));
        assert!(!viewer_may(&C::TerminalInput(v1::TerminalInput::default())));
        assert!(!viewer_may(&C::ApprovalResolve(
            v1::ApprovalResolve::default()
        )));
        assert!(Role::Owner > Role::Member && Role::Member > Role::Viewer);
    }
}
