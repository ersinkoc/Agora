//! Plugins kept by the hub, and what each person allowed them to see (ADR-0114 decision 6, build step 3; docs/ux/plugins.md).
//!
//! - **Installing** is the owner's, on the hub's own machine (`agora hub plugin add FILE`): no protocol message installs or removes a
//!   plugin. The package is checked exactly as a window checks it ([`crate::plugin_package`]) and kept as opaque bytes; the hub never
//!   runs it. Adding a plugin whose id is installed replaces it (another version); the grants people gave stay, each for the version it
//!   was given for, so a version that asks for more is held in each window until its person looks again.
//! - **Windows** list the installed plugins and fetch each package in pieces ([`PIECE`]), then check it again themselves.
//! - **Grants** are kept per person, by plugin id, with the version they were given for and what that version asked for (`seen`, taken
//!   from the installed manifest, never from the window). Removing a plugin forgets everyone's grant for it.
//!
//! The store is read on every request (nothing is cached), so a running hub follows `agora hub plugin add|remove` at once: a window
//! that loads next sees the change.

use agora_protocol::v1::{PluginFetchResult, PluginGrant, PluginGrantSet, PluginInfo};
use agora_protocol::{Error, ErrorCode};
use rusqlite::{params, OptionalExtension};

use crate::plugin_package::{self, Refused};
use crate::store::{write_tx, SqliteStore, StoreError};

/// The most plugins one hub keeps.
pub const MAX_PLUGINS: usize = 16;
/// The largest piece of a package one `PluginFetchResult` carries: well inside a frame (`DEFAULT_MAX_PAYLOAD`, 1 MiB).
pub const PIECE: usize = 512 * 1024;

/// Why a package was not installed.
#[derive(Debug, PartialEq, Eq)]
pub enum AddError {
    /// The package failed the window's checks.
    Refused(Refused),
    /// [`MAX_PLUGINS`] are installed already and this is not one of them.
    Full,
    Store(String),
}

impl From<StoreError> for AddError {
    fn from(e: StoreError) -> Self {
        Self::Store(e.to_string())
    }
}

impl From<rusqlite::Error> for AddError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(e.to_string())
    }
}

/// An installed package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub id: String,
    pub version: String,
    pub name: String,
    pub capabilities: Vec<String>,
    /// The version it replaced, when a plugin with this id was installed.
    pub replaced: Option<String>,
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// Capability names are kept space-separated (none has a space).
fn join(list: &[String]) -> String {
    list.join(" ")
}

fn split(s: &str) -> Vec<String> {
    s.split(' ')
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}

fn internal() -> Error {
    Error::new(ErrorCode::Internal).with("incident_id", "plugins")
}

/// `plugin.not_found` naming the id the window sent, cut to what a manifest id can be (40 characters): the id comes from the request.
fn not_found(id: &str) -> Error {
    let id: String = id.chars().take(40).collect();
    Error::new(ErrorCode::PluginNotFound).with("plugin_id", id.as_str())
}

pub struct Plugins<'a> {
    store: &'a SqliteStore,
}

impl<'a> Plugins<'a> {
    pub fn new(store: &'a SqliteStore) -> Self {
        Self { store }
    }

    /// Checks `package` and installs it for the hub (`agora hub plugin add`), recording it in the audit trail as the owner's, refused or
    /// not. `agora_version` is the hub's own: a plugin that needs a newer Agora is refused, as a window of this release would refuse it.
    pub fn add(&self, package: &[u8], agora_version: &str, now_ms: u64) -> Result<Added, AddError> {
        let result = self.add_checked(package, agora_version, now_ms);
        // The row's place names the plugin (as `user.*` rows name the person); a refused package has none it could name.
        let (target, outcome) = match &result {
            Ok(a) => (format!("{}@{}", a.id, a.version), "ok".to_string()),
            Err(AddError::Refused(r)) => (String::new(), format!("package.{}", r.code.as_str())),
            Err(AddError::Full) => (String::new(), "plugin.full".to_string()),
            Err(AddError::Store(_)) => (String::new(), "internal".to_string()),
        };
        let _ = crate::audit::Audit::new(self.store).record(
            now_ms,
            "owner",
            "plugin.add",
            &target,
            &outcome,
        );
        result
    }

    fn add_checked(
        &self,
        package: &[u8],
        agora_version: &str,
        now_ms: u64,
    ) -> Result<Added, AddError> {
        let checked = plugin_package::check(package, agora_version).map_err(AddError::Refused)?;
        let m = &checked.manifest;
        let mut conn = self.store.lock()?;
        let tx = write_tx(&mut conn)?;
        let replaced: Option<String> = tx
            .query_row("SELECT version FROM plugins WHERE id = ?1", [&m.id], |r| {
                r.get(0)
            })
            .optional()?;
        if replaced.is_none() {
            let n: i64 = tx.query_row("SELECT COUNT(*) FROM plugins", [], |r| r.get(0))?;
            if usize::try_from(n).unwrap_or(usize::MAX) >= MAX_PLUGINS {
                return Err(AddError::Full);
            }
        }
        tx.execute(
            "INSERT INTO plugins (id, version, name, capabilities, sha256, installed_ms, package)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET version = excluded.version, name = excluded.name,
                 capabilities = excluded.capabilities, sha256 = excluded.sha256,
                 installed_ms = excluded.installed_ms, package = excluded.package",
            params![
                m.id,
                m.version,
                m.name,
                join(&m.capabilities),
                checked.sha256,
                to_i64(now_ms),
                package
            ],
        )?;
        tx.commit()?;
        Ok(Added {
            id: m.id.clone(),
            version: m.version.clone(),
            name: m.name.clone(),
            capabilities: m.capabilities.clone(),
            replaced,
        })
    }

    /// Removes a plugin and everyone's grant for it (`agora hub plugin remove`), recorded as the owner's. False when none had that id.
    pub fn remove(&self, id: &str, now_ms: u64) -> Result<bool, StoreError> {
        let removed = {
            let mut conn = self.store.lock()?;
            let tx = write_tx(&mut conn)?;
            let n = tx.execute("DELETE FROM plugins WHERE id = ?1", [id])?;
            tx.execute("DELETE FROM plugin_grants WHERE plugin_id = ?1", [id])?;
            tx.commit()?;
            n > 0
        };
        let outcome = if removed { "ok" } else { "plugin.not_found" };
        let _ = crate::audit::Audit::new(self.store).record(
            now_ms,
            "owner",
            "plugin.remove",
            id,
            outcome,
        );
        Ok(removed)
    }

    /// The installed plugins, by id.
    pub fn list(&self) -> Result<Vec<PluginInfo>, StoreError> {
        let conn = self.store.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, version, name, sha256, length(package), capabilities, installed_ms FROM plugins ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PluginInfo {
                plugin_id: r.get(0)?,
                version: r.get(1)?,
                name: r.get(2)?,
                sha256: r.get(3)?,
                size: to_u64(r.get(4)?),
                capabilities: split(&r.get::<_, String>(5)?),
                installed_ms: to_u64(r.get(6)?),
            })
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    /// A piece of an installed package from `offset`, at most [`PIECE`] bytes. SQLite reads the whole blob under the store
    /// lock to cut the piece out (at most 2 MiB, the largest package); only the piece is copied out and sent. `plugin.not_found` unless
    /// `plugin_id` is installed with the package `sha256`.
    pub fn piece(
        &self,
        plugin_id: &str,
        sha256: &str,
        offset: u64,
    ) -> Result<PluginFetchResult, Error> {
        let conn = self.store.lock().map_err(|_| internal())?;
        let total: Option<i64> = conn
            .query_row(
                "SELECT length(package) FROM plugins WHERE id = ?1 AND sha256 = ?2",
                params![plugin_id, sha256],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| internal())?;
        let total = to_u64(total.ok_or_else(|| not_found(plugin_id))?);
        if offset > total {
            return Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "offset"));
        }
        let len = (total - offset).min(PIECE as u64);
        // SQLite's substr counts a blob's bytes from 1; it loads the whole blob (at most 2 MiB) to cut the piece.
        let data: Vec<u8> = conn
            .query_row(
                "SELECT substr(package, ?3, ?4) FROM plugins WHERE id = ?1 AND sha256 = ?2",
                params![plugin_id, sha256, to_i64(offset + 1), to_i64(len)],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| internal())?
            .ok_or_else(|| not_found(plugin_id))?;
        Ok(PluginFetchResult {
            plugin_id: plugin_id.to_string(),
            sha256: sha256.to_string(),
            total_size: total,
            offset,
            data,
        })
    }

    /// `user`'s grants, by plugin id, each for the version it was given for.
    pub fn grants_of(&self, user: &str) -> Result<Vec<PluginGrant>, StoreError> {
        let conn = self.store.lock()?;
        let mut stmt = conn.prepare(
            "SELECT plugin_id, version, seen, granted, set_ms FROM plugin_grants WHERE user_id = ?1 ORDER BY plugin_id",
        )?;
        let rows = stmt.query_map([user], |r| {
            Ok(PluginGrant {
                plugin_id: r.get(0)?,
                version: r.get(1)?,
                seen: split(&r.get::<_, String>(2)?),
                granted: split(&r.get::<_, String>(3)?),
                set_ms: to_u64(r.get(4)?),
            })
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    /// `user` allows `set.granted` of what the installed version asks for, and has seen all of it. Refused with `plugin.not_found` unless
    /// `set.version` with the package `set.sha256` is the installed one (the person was shown exactly that package), and with `protocol.invalid_message` (`granted`) for a capability that version does not ask for.
    /// `seen` is the installed manifest's own list.
    pub fn set_grant(&self, user: &str, set: &PluginGrantSet, now_ms: u64) -> Result<(), Error> {
        let mut conn = self.store.lock().map_err(|_| internal())?;
        let tx = write_tx(&mut conn).map_err(|_| internal())?;
        let caps: Option<String> = tx
            .query_row(
                "SELECT capabilities FROM plugins WHERE id = ?1 AND version = ?2 AND sha256 = ?3",
                params![set.plugin_id, set.version, set.sha256],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| internal())?;
        let seen = split(&caps.ok_or_else(|| not_found(&set.plugin_id))?);
        if set.granted.len() > seen.len() || set.granted.iter().any(|c| !seen.contains(c)) {
            return Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "granted"));
        }
        // In the manifest's order, each once.
        let granted: Vec<String> = seen
            .iter()
            .filter(|c| set.granted.contains(c))
            .cloned()
            .collect();
        tx.execute(
            "INSERT INTO plugin_grants (user_id, plugin_id, version, seen, granted, set_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(user_id, plugin_id) DO UPDATE SET version = excluded.version, seen = excluded.seen,
                 granted = excluded.granted, set_ms = excluded.set_ms",
            params![
                user,
                set.plugin_id,
                set.version,
                join(&seen),
                join(&granted),
                to_i64(now_ms)
            ],
        )
        .map_err(|_| internal())?;
        tx.commit().map_err(|_| internal())
    }

    /// `user` stops showing a plugin: their grant for it is forgotten (nothing else changes; none is not an error).
    pub fn remove_grant(&self, user: &str, plugin_id: &str) -> Result<(), StoreError> {
        let conn = self.store.lock()?;
        conn.execute(
            "DELETE FROM plugin_grants WHERE user_id = ?1 AND plugin_id = ?2",
            params![user, plugin_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn b64(b: &[u8]) -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in b.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                if i <= c.len() {
                    out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A package of one entry script (and `filler` bytes of a text file, to make it big).
    pub(crate) fn package(id: &str, version: &str, caps: &[&str], filler: usize) -> Vec<u8> {
        let entry = b"agora.watch('agents.read', function () {});";
        let text = vec![b'x'; filler];
        let mut files = format!("\"panel.js\": \"{}\"", hex(&Sha256::digest(entry)));
        if filler > 0 {
            files.push_str(&format!(
                ", \"big.txt\": \"{}\"",
                hex(&Sha256::digest(&text))
            ));
        }
        let caps: Vec<String> = caps.iter().map(|c| format!("\"{c}\"")).collect();
        let manifest = format!(
            r#"{{"id":"{id}","name":"Demo {id}","version":"{version}","author":"Someone","licence":"MIT","min_agora":"0.1.0","panels":[{{"id":"main"}}],"capabilities":[{}],"entry":"panel.js","files":{{{files}}}}}"#,
            caps.join(",")
        );
        let mut bundle = format!(
            r#"{{"agora-plugin-bundle":1,"files":{{"agora-plugin.json":"{}","panel.js":"{}""#,
            b64(manifest.as_bytes()),
            b64(entry)
        );
        if filler > 0 {
            bundle.push_str(&format!(r#","big.txt":"{}""#, b64(&text)));
        }
        bundle.push_str("}}");
        bundle.into_bytes()
    }

    fn store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    #[test]
    fn a_package_is_installed_listed_replaced_and_removed_with_its_grants() {
        let s = store();
        let p = Plugins::new(&s);
        let one = package("demo", "1.0.0", &["agents.read"], 0);
        let a = p.add(&one, "0.3.0", 10).unwrap();
        assert_eq!(
            (a.id.as_str(), a.version.as_str(), a.replaced),
            ("demo", "1.0.0", None)
        );
        let l = p.list().unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].sha256, hex(&Sha256::digest(&one)));
        assert_eq!((l[0].size, l[0].installed_ms), (one.len() as u64, 10));
        assert_eq!(l[0].capabilities, ["agents.read"]);
        p.set_grant(
            "ayse",
            &PluginGrantSet {
                plugin_id: "demo".into(),
                version: "1.0.0".into(),
                granted: vec!["agents.read".into()],
                sha256: hex(&Sha256::digest(&one)),
            },
            11,
        )
        .unwrap();
        // Another version replaces it; the grant stays, for the version it was given for.
        let two = package("demo", "1.1.0", &["agents.read", "usage.read"], 0);
        let b = p.add(&two, "0.3.0", 12).unwrap();
        assert_eq!(b.replaced.as_deref(), Some("1.0.0"));
        assert_eq!(p.list().unwrap()[0].version, "1.1.0");
        let g = p.grants_of("ayse").unwrap();
        assert_eq!(
            (g[0].version.as_str(), g[0].seen.as_slice()),
            ("1.0.0", &["agents.read".to_string()][..])
        );
        assert!(p.remove("demo", 13).unwrap());
        assert!(!p.remove("demo", 14).unwrap());
        assert!(p.list().unwrap().is_empty());
        assert!(
            p.grants_of("ayse").unwrap().is_empty(),
            "removing forgets the grants"
        );
        // Every add and remove is the owner's row in the audit trail.
        let rows = crate::audit::Audit::new(&s).list(0, None, 100).unwrap();
        let mine: Vec<(String, String, String)> = rows
            .iter()
            .filter(|r| r.action.starts_with("plugin."))
            .map(|r| (r.action.clone(), r.node.clone(), r.outcome.clone()))
            .collect();
        let row = |a: &str, n: &str, o: &str| (a.to_string(), n.to_string(), o.to_string());
        assert_eq!(
            mine,
            [
                row("plugin.add", "demo@1.0.0", "ok"),
                row("plugin.add", "demo@1.1.0", "ok"),
                row("plugin.remove", "demo", "ok"),
                row("plugin.remove", "demo", "plugin.not_found"),
            ]
        );
        assert!(rows
            .iter()
            .filter(|r| r.action.starts_with("plugin."))
            .all(|r| r.who == "owner"));
    }

    #[test]
    fn a_bad_package_is_refused_whole_and_audited() {
        let s = store();
        let p = Plugins::new(&s);
        let mut bad = package("demo", "1.0.0", &["agents.read"], 0);
        // One byte of the entry changed: its hash no longer matches.
        let at = bad.len() - 6;
        bad[at] = if bad[at] == b'A' { b'B' } else { b'A' };
        let e = p.add(&bad, "0.3.0", 1).unwrap_err();
        assert!(
            matches!(
                e,
                AddError::Refused(Refused {
                    code: plugin_package::Refusal::Hash | plugin_package::Refusal::Format,
                    ..
                })
            ),
            "{e:?}"
        );
        assert_eq!(
            p.add(&package("demo", "1.0.0", &["net.fetch"], 0), "0.3.0", 2)
                .unwrap_err(),
            AddError::Refused(Refused {
                code: plugin_package::Refusal::Capability,
                detail: Some("net.fetch".into())
            })
        );
        let big = package("big", "1.0.0", &[], plugin_package::MAX_PACKAGE_BYTES);
        assert!(matches!(
            p.add(&big, "0.3.0", 3),
            Err(AddError::Refused(Refused {
                code: plugin_package::Refusal::TooLarge,
                ..
            }))
        ));
        assert!(p.list().unwrap().is_empty());
        let rows = crate::audit::Audit::new(&s).list(0, None, 100).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.action == "plugin.add" && r.outcome == "package.capability"));
    }

    #[test]
    fn the_hub_keeps_at_most_its_cap_but_replaces_one_it_has() {
        let s = store();
        let p = Plugins::new(&s);
        for i in 0..MAX_PLUGINS {
            p.add(&package(&format!("p{i}"), "1.0.0", &[], 0), "0.3.0", 1)
                .unwrap();
        }
        assert_eq!(
            p.add(&package("one-more", "1.0.0", &[], 0), "0.3.0", 2),
            Err(AddError::Full)
        );
        assert!(p.add(&package("p0", "2.0.0", &[], 0), "0.3.0", 3).is_ok());
    }

    #[test]
    fn a_package_is_fetched_in_pieces_by_its_hash() {
        let s = store();
        let p = Plugins::new(&s);
        let big = package("big", "1.0.0", &[], 1_200_000);
        p.add(&big, "0.3.0", 1).unwrap();
        let sha = p.list().unwrap()[0].sha256.clone();
        let mut got = Vec::new();
        loop {
            let piece = p.piece("big", &sha, got.len() as u64).unwrap();
            assert_eq!(piece.total_size, big.len() as u64);
            assert!(piece.data.len() <= PIECE);
            if piece.data.is_empty() {
                break;
            }
            got.extend(piece.data);
        }
        assert_eq!(got, big);
        // Another package, an unknown plugin, or past the end.
        let e = p.piece("big", &"0".repeat(64), 0).unwrap_err();
        assert_eq!(
            (e.code, e.params.contains_key("plugin_id")),
            (ErrorCode::PluginNotFound, true)
        );
        assert_eq!(
            p.piece("nope", &sha, 0).unwrap_err().code,
            ErrorCode::PluginNotFound
        );
        // The id echoed back is cut to what a manifest id can be.
        let long = p.piece(&"x".repeat(5000), &sha, 0).unwrap_err();
        assert_eq!(
            long.params.get("plugin_id"),
            Some(&agora_protocol::ParamValue::Str("x".repeat(40)))
        );
        assert_eq!(
            p.piece("big", &sha, big.len() as u64 + 1).unwrap_err().code,
            ErrorCode::ProtocolInvalidMessage
        );
    }

    /// Removing a person forgets their plugin grants, and nobody else's.
    #[test]
    fn removing_a_person_forgets_their_plugin_grants() {
        use crate::users::{Role, Users};
        let s = store();
        let p = Plugins::new(&s);
        let bytes = package("demo", "1.0.0", &["agents.read"], 0);
        p.add(&bytes, "0.3.0", 1).unwrap();
        let users = Users::new(&s);
        let eda = users.add("Eda", Role::Viewer, 2).unwrap();
        let bo = users.add("Bo", Role::Member, 3).unwrap();
        for who in [&eda.id, &bo.id] {
            p.set_grant(
                who,
                &PluginGrantSet {
                    plugin_id: "demo".into(),
                    version: "1.0.0".into(),
                    granted: vec!["agents.read".into()],
                    sha256: hex(&Sha256::digest(&bytes)),
                },
                4,
            )
            .unwrap();
        }
        assert!(users.remove(&eda.id).unwrap());
        assert!(p.grants_of(&eda.id).unwrap().is_empty());
        assert_eq!(p.grants_of(&bo.id).unwrap().len(), 1);
    }

    #[test]
    fn grants_are_per_person_and_checked_against_the_installed_version() {
        let s = store();
        let p = Plugins::new(&s);
        let bytes = package("demo", "1.0.0", &["agents.read", "theme.read"], 0);
        p.add(&bytes, "0.3.0", 1).unwrap();
        let sha = hex(&Sha256::digest(&bytes));
        let set_for = |user: &str, version: &str, granted: &[&str], sha: &str| {
            p.set_grant(
                user,
                &PluginGrantSet {
                    plugin_id: "demo".into(),
                    version: version.into(),
                    granted: granted.iter().map(|c| c.to_string()).collect(),
                    sha256: sha.into(),
                },
                5,
            )
        };
        let set =
            |user: &str, version: &str, granted: &[&str]| set_for(user, version, granted, &sha);
        // Not the package the person was shown (the owner added another one under the same version meanwhile).
        assert_eq!(
            set_for("ayse", "1.0.0", &[], &"0".repeat(64))
                .unwrap_err()
                .code,
            ErrorCode::PluginNotFound
        );
        set(
            "ayse",
            "1.0.0",
            &["theme.read", "agents.read", "theme.read"],
        )
        .unwrap_err();
        set("ayse", "1.0.0", &["theme.read", "agents.read"]).unwrap();
        set("bora", "1.0.0", &[]).unwrap();
        // Not the installed version, or a capability it does not ask for.
        assert_eq!(
            set("ayse", "0.9.0", &[]).unwrap_err().code,
            ErrorCode::PluginNotFound
        );
        assert_eq!(
            set("ayse", "1.0.0", &["chat.read"]).unwrap_err().code,
            ErrorCode::ProtocolInvalidMessage
        );
        let a = p.grants_of("ayse").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].seen, ["agents.read", "theme.read"]);
        assert_eq!(
            a[0].granted,
            ["agents.read", "theme.read"],
            "in the manifest's order"
        );
        assert_eq!(
            p.grants_of("bora").unwrap()[0].granted,
            Vec::<String>::new()
        );
        assert!(p.grants_of("owner").unwrap().is_empty());
        p.remove_grant("bora", "demo").unwrap();
        assert!(p.grants_of("bora").unwrap().is_empty());
        assert_eq!(
            p.grants_of("ayse").unwrap().len(),
            1,
            "one person's choice is theirs alone"
        );
    }
}
