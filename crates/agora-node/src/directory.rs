//! Selected directories and portable project identity (ADR-0052). No Git mutation on directory launches.
use agora_core::ProjectId;
use agora_protocol::{v1, Error, ErrorCode};
use std::io::Write;
use std::path::PathBuf;

pub const METADATA: &str = ".agent-project.json";

pub fn invalid(field: &'static str) -> Error {
    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
}

pub fn inspect(
    path: &str,
    node: &str,
    create_identity: bool,
) -> Result<(v1::DirectoryInfo, ProjectId), Error> {
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Err(invalid("directory.path"));
    }
    let path = std::fs::canonicalize(path).map_err(|_| invalid("directory.path"))?;
    if !path.is_dir() {
        return Err(invalid("directory.path"));
    }
    let checkout = crate::git::discover_checkout(&path).ok();
    let root = checkout
        .as_ref()
        .map_or(path.as_path(), |c| c.root.as_path());
    let metadata = root.join(METADATA);
    if create_identity && !metadata.exists() {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).map_err(|_| invalid("directory.randomness"))?;
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let id = format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        );
        let text = serde_json::to_vec_pretty(&serde_json::json!({"project_id": id}))
            .map_err(|_| invalid("directory.metadata"))?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&metadata)
        {
            Ok(mut f) => {
                if f.write_all(&text).and_then(|()| f.sync_all()).is_err() {
                    let _ = std::fs::remove_file(&metadata);
                    return Err(invalid("directory.metadata"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(invalid("directory.metadata")),
        }
    }
    let portable = match std::fs::symlink_metadata(&metadata) {
        Ok(meta) => {
            if !meta.is_file() || meta.len() > 4096 {
                return Err(invalid("directory.metadata"));
            }
            let bytes = std::fs::read(&metadata).map_err(|_| invalid("directory.metadata"))?;
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| invalid("directory.metadata"))?;
            let id = value
                .get("project_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| invalid("directory.metadata"))?;
            Some(ProjectId::portable(id).map_err(|_| invalid("directory.project_id"))?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(invalid("directory.metadata")),
    };
    let is_portable = portable.is_some();
    let project = portable.unwrap_or_else(|| {
        checkout.as_ref().map_or_else(
            || ProjectId::directory(node, &path.to_string_lossy()),
            |c| c.project.clone(),
        )
    });
    let branch = checkout
        .as_ref()
        .and_then(|c| crate::git::status(&c.root).ok())
        .map_or(String::new(), |_| {
            std::process::Command::new("git")
                .args(["-C"])
                .arg(root)
                .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map_or(String::new(), |o| {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                })
        });
    Ok((
        v1::DirectoryInfo {
            path: path.to_string_lossy().into_owned(),
            project_id: project.to_string(),
            git_root: checkout.map_or(String::new(), |c| c.root.to_string_lossy().into_owned()),
            branch,
            portable: is_portable,
            previous_project_id: String::new(),
        },
        project,
    ))
}

/// Persisted owner-confirmed associations. Paths never silently switch logical project after metadata changes.
#[derive(Default)]
pub struct Bindings {
    path: PathBuf,
    ids: std::collections::BTreeMap<String, String>,
    valid: bool,
}
impl Bindings {
    pub fn load(path: PathBuf) -> Self {
        let (ids, valid) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(ids) => (ids, true),
                Err(_) => (Default::default(), false),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Default::default(), true),
            Err(_) => (Default::default(), false),
        };
        Self { path, ids, valid }
    }
    pub fn previous(&self, info: &v1::DirectoryInfo) -> String {
        self.ids
            .get(&info.path)
            .filter(|p| *p != &info.project_id)
            .cloned()
            .unwrap_or_default()
    }
    pub fn confirm(&mut self, info: &v1::DirectoryInfo) -> Result<(), Error> {
        if !self.valid {
            return Err(invalid("directory.bindings"));
        }
        let mut ids = self.ids.clone();
        ids.insert(info.path.clone(), info.project_id.clone());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| invalid("directory.bindings"))?;
        }
        let bytes = serde_json::to_vec(&ids).map_err(|_| invalid("directory.bindings"))?;
        agora_platform::write_private(&self.path, &bytes)
            .map_err(|_| invalid("directory.bindings"))?;
        self.ids = ids;
        Ok(())
    }
}

/// Canonical signature format: domain + NUL-separated UTF-8 fields; no field may contain NUL.
#[allow(clippy::too_many_arguments)] // The arguments are the independently bound canonical wire fields.
pub fn signed_message(
    node: &str,
    operation: &str,
    path: &str,
    kind: &str,
    project: &str,
    option: bool,
    identity_change: bool,
    nonce: &str,
    expires: u64,
) -> Vec<u8> {
    let fields = [
        node,
        operation,
        path,
        kind,
        project,
        if option { "1" } else { "0" },
        if identity_change { "1" } else { "0" },
        nonce,
    ];
    let mut out = b"directory-action-v1".to_vec();
    for f in fields {
        out.push(0);
        out.extend_from_slice(f.as_bytes());
    }
    out.push(0);
    out.extend_from_slice(expires.to_string().as_bytes());
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("directory-id-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn portable_identity_is_opt_in_shared_across_nodes_and_existing_metadata_is_never_overwritten()
    {
        let a = temp("a");
        let b = temp("b");
        let first = inspect(a.to_str().unwrap(), "node-a", false).unwrap();
        assert!(!a.join(METADATA).exists());
        assert!(first.0.git_root.is_empty());
        assert!(first.1.root_commit().is_empty());
        let created = inspect(a.to_str().unwrap(), "node-a", true).unwrap();
        let original = std::fs::read(a.join(METADATA)).unwrap();
        std::fs::copy(a.join(METADATA), b.join(METADATA)).unwrap();
        let clone = inspect(b.to_str().unwrap(), "node-b", false).unwrap();
        assert_eq!(created.0.project_id, clone.0.project_id);
        assert!(created.0.portable);
        inspect(a.to_str().unwrap(), "node-a", true).unwrap();
        assert_eq!(std::fs::read(a.join(METADATA)).unwrap(), original);
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }
    #[test]
    fn an_existing_binding_never_silently_changes_identity_and_corrupt_state_fails_closed() {
        let p = temp("binding");
        let file = p.join("bindings.json");
        let mut b = Bindings::load(file.clone());
        let info = v1::DirectoryInfo {
            path: "/work".into(),
            project_id: "first".into(),
            ..Default::default()
        };
        b.confirm(&info).unwrap();
        let changed = v1::DirectoryInfo {
            project_id: "second".into(),
            ..info.clone()
        };
        assert_eq!(Bindings::load(file.clone()).previous(&changed), "first");
        std::fs::write(&file, b"invalid").unwrap();
        assert!(Bindings::load(file).confirm(&changed).is_err());
        let _ = std::fs::remove_dir_all(p);
    }
}
