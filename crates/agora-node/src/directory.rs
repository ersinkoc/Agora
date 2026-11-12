//! Selected directories and portable project identity (ADR-0052). No Git mutation on directory launches.
use agora_core::ProjectId;
use agora_protocol::{v1, Error, ErrorCode};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    let path = agora_platform::canonicalize_plain(std::path::Path::new(path))
        .map_err(|_| invalid("directory.path"))?;
    if !path.is_dir() {
        return Err(invalid("directory.path"));
    }
    // Git and the OS may spell the same root differently (Windows extended paths, macOS temp aliases).
    // Use canonical paths on both sides before computing a selected subdirectory's relative path.
    let checkout = match crate::git::discover_checkout(&path) {
        Ok(mut checkout) => {
            checkout.root = agora_platform::canonicalize_plain(&checkout.root)
                .map_err(|_| invalid("directory.git_root"))?;
            Some(checkout)
        }
        Err(_) => None,
    };
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
            crate::git::hardened(root)
                .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
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

/// Most folders one `directory.list` answer carries (ADR-0053); more are reported as `truncated`.
pub const MAX_LISTED: usize = 500;

/// Where a folder listing may look and what it must keep out of sight: the user's home folder (an empty `path` means it) and the
/// node's own folders (ADR-0052's refusals, [`crate::sandbox::private_folder`]). Canonical paths as `std::fs::canonicalize` spells
/// them, which is how the folder being listed is compared with them.
pub struct ListScope<'a> {
    pub home: Option<&'a Path>,
    pub node_home: &'a Path,
    pub worktree_root: &'a Path,
    /// A slow volume answers with what it listed by then (`truncated`).
    pub budget: std::time::Duration,
}

/// The folders directly inside `requested` (ADR-0053 `directory.list`): names and whether each holds a `.git`, sorted, at most
/// [`MAX_LISTED`], never a file, a link, a secret folder or the node's own folder. The folder is listed through the handle that was
/// opened and checked to be the folder asked for, so a folder swapped for a link in between cannot redirect the listing (#326, #333).
pub fn list(requested: &str, scope: &ListScope<'_>) -> Result<v1::DirectoryEntries, Error> {
    if requested.len() > 4096 || requested.contains('\0') {
        return Err(invalid("directory.path"));
    }
    let asked = if requested.is_empty() {
        scope
            .home
            .map(Path::to_path_buf)
            .ok_or_else(|| invalid("directory.path"))?
    } else {
        PathBuf::from(requested)
    };
    if !asked.is_absolute() {
        return Err(invalid("directory.path"));
    }
    // Everything below is decided on the folder the open handle holds, never on a second look at the path: a link swapped in
    // between two looks could otherwise pass the check on one folder and list another.
    let real = std::fs::canonicalize(&asked).map_err(|_| invalid("directory.path"))?;
    let handle = agora_platform::open_dir(&real).map_err(|_| invalid("directory.path"))?;
    let opened = agora_platform::opened_path(&handle).map_err(|_| invalid("directory.path"))?;
    if opened != real {
        return Err(invalid("directory.path"));
    }
    if crate::sandbox::private_folder(&opened, scope.home, scope.node_home, scope.worktree_root) {
        return Err(invalid("directory.protected"));
    }
    let end = std::time::Instant::now() + scope.budget;
    let mut names = Vec::new();
    let mut truncated = false;
    agora_platform::visit_dir(&handle, |item: agora_platform::DirItem| {
        if names.len() >= MAX_LISTED || std::time::Instant::now() > end {
            truncated = true;
            return std::ops::ControlFlow::Break(());
        }
        // `is_dir` does not follow links: a link to a folder is not listed.
        if item.is_dir
            && !crate::sandbox::private_folder(
                &opened.join(&item.name),
                scope.home,
                scope.node_home,
                scope.worktree_root,
            )
        {
            names.push(item.name);
        }
        std::ops::ControlFlow::Continue(())
    })
    .map_err(|_| invalid("directory.path"))?;
    names.sort();
    let entries = names
        .into_iter()
        .map(|name| v1::DirectoryEntry {
            is_git: std::fs::symlink_metadata(opened.join(&name).join(".git")).is_ok(),
            name: name.to_string_lossy().into_owned(),
        })
        .collect();
    // Spelled the way people and programs expect (no `\\?\\` on Windows), as `directory.inspect` answers.
    let shown = agora_platform::canonicalize_plain(&opened).unwrap_or_else(|_| opened.clone());
    Ok(v1::DirectoryEntries {
        path: shown.to_string_lossy().into_owned(),
        parent: shown
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        entries,
        truncated,
    })
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
    /// A home folder with secret folders, the node's own folder (holding its worktrees) and ordinary folders; canonical paths.
    struct Home {
        home: PathBuf,
        node: PathBuf,
        worktrees: PathBuf,
    }
    fn home(name: &str) -> Home {
        let home = std::fs::canonicalize(temp(name)).unwrap();
        for d in [
            ".ssh",
            ".aws",
            ".config/gh",
            ".config/nvim",
            "code/app/.git",
            "code/notes",
            ".agora/worktrees/agent-1",
        ] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join("code/readme.txt"), "a file").unwrap();
        std::fs::write(home.join("plain.txt"), "a file").unwrap();
        Home {
            node: home.join(".agora"),
            worktrees: home.join(".agora/worktrees"),
            home,
        }
    }
    fn scope(h: &Home) -> ListScope<'_> {
        ListScope {
            home: Some(&h.home),
            node_home: &h.node,
            worktree_root: &h.worktrees,
            budget: std::time::Duration::from_secs(5),
        }
    }
    fn names(e: &v1::DirectoryEntries) -> Vec<(String, bool)> {
        e.entries
            .iter()
            .map(|x| (x.name.clone(), x.is_git))
            .collect()
    }

    #[test]
    fn a_listing_has_folders_only_sorted_with_git_marked_and_secrets_hidden() {
        let h = home("list-home");
        // An empty path is the home folder: no secret folder, no node folder, no file.
        let top = list("", &scope(&h)).unwrap();
        assert_eq!(top.path, h.home.to_string_lossy());
        assert_eq!(top.parent, h.home.parent().unwrap().to_string_lossy());
        assert_eq!(
            names(&top),
            [(".config".to_string(), false), ("code".to_string(), false)]
        );
        assert!(!top.truncated);
        // One level down: only what is not secret there either; a folder holding `.git` is marked.
        let config = list(&h.home.join(".config").to_string_lossy(), &scope(&h)).unwrap();
        assert_eq!(names(&config), [("nvim".to_string(), false)]);
        let code = list(&h.home.join("code").to_string_lossy(), &scope(&h)).unwrap();
        assert_eq!(
            names(&code),
            [("app".to_string(), true), ("notes".to_string(), false)]
        );
        // A secret folder or the node's own is refused outright; its worktrees are fine.
        for refused in [
            h.home.join(".ssh"),
            h.home.join(".config/gh"),
            h.node.clone(),
        ] {
            let e = list(&refused.to_string_lossy(), &scope(&h)).unwrap_err();
            assert_eq!(
                format!("{:?}", e.params.get("field")),
                format!(
                    "{:?}",
                    Some(agora_protocol::ParamValue::from("directory.protected"))
                ),
                "{refused:?}"
            );
        }
        assert_eq!(
            names(&list(&h.worktrees.to_string_lossy(), &scope(&h)).unwrap()),
            [("agent-1".to_string(), false)]
        );
        // A relative path, a file and a missing folder are refused.
        for bad in [
            "code",
            &*h.home.join("plain.txt").to_string_lossy(),
            &*h.home.join("missing").to_string_lossy(),
        ] {
            assert!(list(bad, &scope(&h)).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&h.home);
    }

    #[test]
    fn a_big_folder_answers_500_and_says_there_are_more() {
        let h = home("list-big");
        let big = h.home.join("big");
        for i in 0..520 {
            std::fs::create_dir_all(big.join(format!("d{i:03}"))).unwrap();
        }
        let e = list(&big.to_string_lossy(), &scope(&h)).unwrap();
        assert_eq!(e.entries.len(), MAX_LISTED);
        assert!(e.truncated);
        let mut sorted = names(&e);
        sorted.sort();
        assert_eq!(names(&e), sorted);
        let _ = std::fs::remove_dir_all(&h.home);
    }

    #[test]
    fn a_link_is_not_listed_and_a_secret_folder_never_shows_through_one_swapped_in() {
        let h = home("list-links");
        let secret = h.home.join(".ssh");
        std::fs::create_dir_all(secret.join("KEYS")).unwrap();
        let code = h.home.join("code");
        if agora_platform::symlink(&secret, &code.join("link")).is_err() {
            let _ = std::fs::remove_dir_all(&h.home);
            return; // no symbolic links here (Windows without the privilege)
        }
        let e = list(&code.to_string_lossy(), &scope(&h)).unwrap();
        assert!(!names(&e).iter().any(|(n, _)| n == "link"), "{e:?}");
        // Asked for directly, the link resolves to the secret folder and is refused.
        assert!(list(&code.join("link").to_string_lossy(), &scope(&h)).is_err());
        // `d` flips between a folder of its own and a link to `.ssh`: whichever moment the listing hits, `.ssh` is never listed,
        // whether the link is there when the path is checked (refused as secret) or swapped in after it (the open handle is not
        // the folder that was checked).
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flipper = {
            let (stop, root, secret) = (stop.clone(), code.clone(), secret.clone());
            std::thread::spawn(move || {
                let (real, link, name) = (root.join("d.real"), root.join("d.link"), root.join("d"));
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = std::fs::create_dir_all(real.join("mine"));
                    let _ = std::fs::remove_file(&name);
                    let _ = std::fs::rename(&real, &name);
                    let _ = agora_platform::symlink(&secret, &link);
                    let _ = std::fs::remove_dir_all(&name);
                    let _ = std::fs::rename(&link, &name);
                }
            })
        };
        let target = code.join("d").to_string_lossy().into_owned();
        let (mut leaked, mut own) = (0, 0);
        for _ in 0..3000 {
            match list(&target, &scope(&h)) {
                Ok(e) if names(&e).iter().any(|(n, _)| n == "KEYS") => leaked += 1,
                Ok(_) => own += 1,
                Err(_) => {}
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        flipper.join().unwrap();
        let _ = std::fs::remove_dir_all(&h.home);
        assert_eq!(
            leaked, 0,
            "a listing showed a secret folder through a swapped link"
        );
        assert!(own > 0, "the folder of its own was never listed");
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
    fn a_selected_folder_is_reported_in_the_spelling_programs_can_start_in() {
        // On Windows `canonicalize` answers `\\?\C:\...`, and `cmd.exe` started there runs in `C:\Windows` instead.
        let a = temp("plain");
        let (info, _) = inspect(a.to_str().unwrap(), "node-a", false).unwrap();
        assert!(!info.path.starts_with(r"\\?\"), "{}", info.path);
        let _ = std::fs::remove_dir_all(a);
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
