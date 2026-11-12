//! Files of one worktree: lazy listing, bounded reads, lease-checked writes, and change watching.
//!
//! Every path from a client is relative to the worktree root and is checked twice: lexically (no absolute
//! paths, no `..`) and after resolving symlinks (the real location must stay inside the root). Writes go
//! through the project's [`LeaseTable`]. Watching polls modification times, which needs no extra dependency
//! and behaves the same on every OS; a native watcher can replace it behind the same API.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use agora_core::{AgentId, LeaseTable};
use agora_protocol::{Error, ErrorCode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesError {
    NotFound(String),
    TooLarge {
        path: String,
        limit: u64,
    },
    /// `..`, an absolute path, or a symlink leading out of the worktree.
    OutsideRoot(String),
    LeaseHeld {
        path: String,
        holder: AgentId,
        expires_at_ms: u64,
    },
    /// The OS failed; the text is for logs, never for users.
    Io(String),
}

impl FilesError {
    /// Map to the protocol error `{ code, params }` at the boundary.
    pub fn to_protocol(&self) -> Error {
        match self {
            Self::NotFound(p) => Error::new(ErrorCode::FilesNotFound).with("path", p.as_str()),
            Self::TooLarge { path, limit } => Error::new(ErrorCode::FilesTooLarge)
                .with("path", path.as_str())
                .with("limit", i64::try_from(*limit).unwrap_or(i64::MAX)),
            Self::OutsideRoot(p) => {
                Error::new(ErrorCode::FilesOutsideRoot).with("path", p.as_str())
            }
            Self::LeaseHeld {
                path,
                holder,
                expires_at_ms,
            } => Error::new(ErrorCode::LeaseHeld)
                .with("path", path.as_str())
                .with("holder_agent_id", holder.as_str())
                .with(
                    "expires_at",
                    i64::try_from(*expires_at_ms).unwrap_or(i64::MAX),
                ),
            Self::Io(_) => Error::new(ErrorCode::Internal).with("incident_id", "unassigned"),
        }
    }
}

impl std::fmt::Display for FilesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_protocol())
    }
}

impl std::error::Error for FilesError {}

fn io_err(path: &str, e: &std::io::Error) -> FilesError {
    if e.kind() == std::io::ErrorKind::NotFound {
        FilesError::NotFound(path.to_string())
    } else {
        FilesError::Io(format!("{path}: {e}"))
    }
}

/// Disk used by a directory tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskUsage {
    pub bytes: u64,
    pub files: u64,
    /// The walk stopped at the entry cap; the numbers are a lower bound.
    pub truncated: bool,
}

/// What [`Files::untracked`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Untracked {
    File {
        data: Vec<u8>,
        executable: bool,
    },
    /// A symbolic link: its target as git stores it (the bytes of the path it names).
    Link(Vec<u8>),
}

/// Total size of regular files under `root` (symlinks are not followed or counted). Includes `.git` data,
/// which is part of what a worktree costs.
pub fn disk_usage(root: &Path) -> std::io::Result<DiskUsage> {
    let mut usage = DiskUsage {
        bytes: 0,
        files: 0,
        truncated: false,
    };
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let Ok(entry) = entry else { continue };
            seen += 1;
            if seen > MAX_ENTRIES {
                usage.truncated = true;
                return Ok(usage);
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(meta) = entry.metadata() {
                    usage.bytes += meta.len();
                    usage.files += 1;
                }
            }
        }
    }
    Ok(usage)
}

/// One directory entry. Names only: contents are listed lazily, one directory at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Access to the files of one worktree.
#[derive(Debug, Clone)]
pub struct Files {
    root: PathBuf,
    max_read: u64,
}

impl Files {
    /// `root` must exist; `max_read` is the largest file `read` returns, in bytes.
    pub fn open(root: &Path, max_read: u64) -> Result<Self, FilesError> {
        let root = fs::canonicalize(root).map_err(|e| io_err(&root.display().to_string(), &e))?;
        Ok(Self { root, max_read })
    }

    /// Resolve a client path to a real path inside the root.
    fn resolve(&self, rel: &str) -> Result<PathBuf, FilesError> {
        let outside = || FilesError::OutsideRoot(rel.to_string());
        let mut clean = PathBuf::new();
        for c in Path::new(rel).components() {
            match c {
                Component::Normal(p) => clean.push(p),
                Component::CurDir => {}
                // `..`, `/x`, `C:\x`: never allowed, even when they would resolve inside the root.
                _ => return Err(outside()),
            }
        }
        let full = self.root.join(&clean);
        // Resolve symlinks on the deepest part that exists, so a link pointing out is caught for reads
        // and for writes to files that do not exist yet.
        let mut existing = full.as_path();
        let mut tail = Vec::new();
        let real = loop {
            match fs::canonicalize(existing) {
                Ok(p) => break p,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => match existing.parent() {
                    Some(parent) if parent.starts_with(&self.root) => {
                        if let Some(name) = existing.file_name() {
                            tail.push(name.to_os_string());
                        }
                        existing = parent;
                    }
                    _ => return Err(outside()),
                },
                Err(e) => return Err(io_err(rel, &e)),
            }
        };
        if !real.starts_with(&self.root) {
            return Err(outside());
        }
        let mut out = real;
        out.extend(tail.iter().rev());
        Ok(out)
    }

    /// Entries of one directory, directories first, then by name.
    pub fn list(&self, rel: &str) -> Result<Vec<Entry>, FilesError> {
        let dir = self.resolve(rel)?;
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| io_err(rel, &e))? {
            let Ok(entry) = entry else { continue };
            let Ok(meta) = entry.metadata() else { continue };
            out.push(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
            });
        }
        out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    /// File contents, refusing files above the read limit before reading them.
    ///
    /// The agent working here can swap the file, or a folder above it, for a link between [`Self::resolve`]'s check and the open,
    /// so where the open file really is is checked again on the open handle itself, and everything after reads that handle.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>, FilesError> {
        self.read_file(rel).map(|(data, _)| data)
    }

    /// [`Self::read`], with whether the file is executable (from the same open handle).
    pub fn read_file(&self, rel: &str) -> Result<(Vec<u8>, bool), FilesError> {
        use std::io::Read;
        let path = self.resolve(rel)?;
        // Without waiting: a FIFO swapped in for the file would block the open until something writes to it.
        let file = agora_platform::open_read(&path).map_err(|e| io_err(rel, &e))?;
        let real = agora_platform::opened_path(&file).map_err(|e| io_err(rel, &e))?;
        if !real.starts_with(&self.root) {
            return Err(FilesError::OutsideRoot(rel.to_string()));
        }
        let meta = file.metadata().map_err(|e| io_err(rel, &e))?;
        if !meta.is_file() {
            return Err(FilesError::NotFound(rel.to_string()));
        }
        if meta.len() > self.max_read {
            return Err(FilesError::TooLarge {
                path: rel.to_string(),
                limit: self.max_read,
            });
        }
        let mut data = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
        // Bounded too: the file can grow after its size was read. One byte past the limit tells a grown file from one that fits.
        file.take(self.max_read.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|e| io_err(rel, &e))?;
        if data.len() as u64 > self.max_read {
            return Err(FilesError::TooLarge {
                path: rel.to_string(),
                limit: self.max_read,
            });
        }
        Ok((data, agora_platform::is_executable(&meta)))
    }

    /// An untracked file for a diff: a link as a link (its target, as git shows one, never what it points at), anything else as
    /// [`Self::read_file`] reads it (regular files only, checked on the open handle). The link's folder is checked before and
    /// after reading it, so a folder swapped for a link in between gives nothing.
    pub fn untracked(&self, rel: &str) -> Result<Untracked, FilesError> {
        let rel_path = Path::new(rel);
        let name = rel_path
            .file_name()
            .ok_or_else(|| FilesError::OutsideRoot(rel.to_string()))?;
        let parent_rel = rel_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = self.resolve(&parent_rel)?;
        let entry = parent.join(name);
        let is_link = fs::symlink_metadata(&entry)
            .map_err(|e| io_err(rel, &e))?
            .file_type()
            .is_symlink();
        if !is_link {
            let (data, executable) = self.read_file(rel)?;
            return Ok(Untracked::File { data, executable });
        }
        let target = fs::read_link(&entry).map_err(|e| io_err(rel, &e))?;
        if fs::canonicalize(&parent).ok().as_deref() != Some(parent.as_path()) {
            return Err(FilesError::OutsideRoot(rel.to_string()));
        }
        let bytes = target.as_os_str().as_encoded_bytes().to_vec();
        if bytes.len() as u64 > self.max_read {
            return Err(FilesError::TooLarge {
                path: rel.to_string(),
                limit: self.max_read,
            });
        }
        Ok(Untracked::Link(bytes))
    }

    /// Write a file on behalf of `agent`, unless another agent holds a live lease covering it.
    /// Missing parent directories are created.
    pub fn write(
        &self,
        rel: &str,
        bytes: &[u8],
        agent: &AgentId,
        leases: &LeaseTable,
        now_ms: u64,
    ) -> Result<(), FilesError> {
        let path = self.resolve(rel)?;
        if let Some(l) = leases.blocking_lease(rel, agent, now_ms) {
            return Err(FilesError::LeaseHeld {
                path: rel.to_string(),
                holder: l.holder.clone(),
                expires_at_ms: l.expires_at_ms,
            });
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(rel, &e))?;
        }
        fs::write(&path, bytes).map_err(|e| io_err(rel, &e))
    }

    /// Start watching: the first [`Watcher::poll`] reports nothing, later polls report what changed.
    pub fn watch(&self) -> Watcher {
        let mut w = Watcher {
            root: self.root.clone(),
            seen: BTreeMap::new(),
        };
        w.seen = w.scan();
        w
    }
}

/// Polling change detector. One `poll` returns one batch, ready to become a `FilesChanged` event.
#[derive(Debug)]
pub struct Watcher {
    root: PathBuf,
    seen: BTreeMap<String, (SystemTime, u64)>,
}

/// Directories never reported (git's own bookkeeping changes constantly and is not project content).
const SKIP_DIRS: &[&str] = &[".git"];
/// Safety valve for enormous trees.
const MAX_ENTRIES: usize = 50_000;

impl Watcher {
    fn scan(&self) -> BTreeMap<String, (SystemTime, u64)> {
        let mut out = BTreeMap::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                if out.len() >= MAX_ENTRIES {
                    return out;
                }
                let Ok(ft) = entry.file_type() else { continue };
                let name = entry.file_name();
                if ft.is_dir() {
                    if !SKIP_DIRS.iter().any(|s| name == *s) {
                        stack.push(entry.path());
                    }
                } else if let Ok(meta) = entry.metadata() {
                    let Ok(rel) = entry.path().strip_prefix(&self.root).map(Path::to_path_buf)
                    else {
                        continue;
                    };
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    out.insert(
                        rel,
                        (
                            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                            meta.len(),
                        ),
                    );
                }
            }
        }
        out
    }

    /// Paths (relative to the root, `/`-separated, sorted) that were added, changed, or removed since the
    /// previous call.
    pub fn poll(&mut self) -> Vec<String> {
        let now = self.scan();
        let mut changed: Vec<String> = now
            .iter()
            .filter(|(p, v)| self.seen.get(*p) != Some(v))
            .map(|(p, _)| p.clone())
            .collect();
        changed.extend(self.seen.keys().filter(|p| !now.contains_key(*p)).cloned());
        changed.sort();
        self.seen = now;
        changed
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tmp() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "agora-files-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn setup() -> (PathBuf, Files) {
        let d = tmp();
        let root = d.join("wt");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "fn a() {}").unwrap();
        fs::write(root.join("README.md"), "hi").unwrap();
        fs::write(d.join("secret.txt"), "top secret").unwrap();
        let files = Files::open(&root, 1024).unwrap();
        (d, files)
    }

    #[test]
    fn lists_one_level_directories_first() {
        let (_d, f) = setup();
        let names: Vec<_> = f
            .list("")
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        assert_eq!(
            names,
            vec![("src".into(), true), ("README.md".into(), false)]
        );
        let sub = f.list("src").unwrap();
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0].size, 9);
    }

    #[test]
    fn disk_usage_counts_regular_files_and_ignores_symlinks() {
        let (d, f) = setup();
        let u = disk_usage(&d.join("wt")).unwrap();
        assert_eq!((u.bytes, u.files, u.truncated), (9 + 2, 2, false));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.join("secret.txt"), d.join("wt/link")).unwrap();
            assert_eq!(disk_usage(&d.join("wt")).unwrap().bytes, 11);
        }
        f.write(
            "big.bin",
            &[0u8; 1000],
            &AgentId::new("a"),
            &LeaseTable::new(),
            0,
        )
        .unwrap();
        assert_eq!(disk_usage(&d.join("wt")).unwrap().bytes, 1011);
    }

    #[test]
    fn reads_and_reports_missing_files() {
        let (_d, f) = setup();
        assert_eq!(f.read("src/a.rs").unwrap(), b"fn a() {}");
        assert_eq!(
            f.read("nope.txt"),
            Err(FilesError::NotFound("nope.txt".into()))
        );
    }

    #[test]
    fn large_files_are_refused_without_being_read() {
        let (d, _) = setup();
        let f = Files::open(&d.join("wt"), 4).unwrap();
        let e = f.read("src/a.rs").unwrap_err();
        assert_eq!(
            e,
            FilesError::TooLarge {
                path: "src/a.rs".into(),
                limit: 4
            }
        );
        assert_eq!(e.to_protocol().code, ErrorCode::FilesTooLarge);
    }

    #[test]
    fn dot_dot_and_absolute_paths_are_rejected() {
        let (d, f) = setup();
        for bad in ["../secret.txt", "src/../../secret.txt", "src/../README.md"] {
            assert!(
                matches!(f.read(bad), Err(FilesError::OutsideRoot(_))),
                "{bad}"
            );
            assert!(
                matches!(f.list(bad), Err(FilesError::OutsideRoot(_))),
                "{bad}"
            );
        }
        let abs = d.join("secret.txt").display().to_string();
        assert!(matches!(f.read(&abs), Err(FilesError::OutsideRoot(_))));
        let agent = AgentId::new("a");
        let leases = LeaseTable::new();
        assert!(matches!(
            f.write("../evil.txt", b"x", &agent, &leases, 0),
            Err(FilesError::OutsideRoot(_))
        ));
        assert!(!d.join("evil.txt").exists());
    }

    #[test]
    fn a_file_swapped_for_a_link_after_the_check_is_not_read() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let (d, f) = setup();
        let root = d.join("wt");
        if agora_platform::symlink(&d.join("secret.txt"), &root.join("probe")).is_err() {
            return; // no symbolic links here (Windows without the privilege)
        }
        let stop = Arc::new(AtomicBool::new(false));
        // The agent flips `notes.md` between a file of its own and a link to a secret outside its worktree.
        let flipper = {
            let (stop, root, secret) = (Arc::clone(&stop), root.clone(), d.join("secret.txt"));
            std::thread::spawn(move || {
                let (file, link, name) = (
                    root.join("n.tmp"),
                    root.join("l.tmp"),
                    root.join("notes.md"),
                );
                while !stop.load(Ordering::Relaxed) {
                    let _ = fs::write(&file, "notes");
                    let _ = fs::rename(&file, &name);
                    let _ = agora_platform::symlink(&secret, &link);
                    let _ = fs::rename(&link, &name);
                }
            })
        };
        let mut leaked = 0;
        for _ in 0..20_000 {
            if let Ok(data) = f.read("notes.md") {
                if data == b"top secret" {
                    leaked += 1;
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
        flipper.join().unwrap();
        assert_eq!(leaked, 0, "the node read the secret through a swapped link");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_leading_out_are_rejected_for_read_list_and_write() {
        let (d, f) = setup();
        let root = d.join("wt");
        std::os::unix::fs::symlink(d.join("secret.txt"), root.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&d, root.join("outdir")).unwrap();
        assert!(matches!(
            f.read("link.txt"),
            Err(FilesError::OutsideRoot(_))
        ));
        assert!(matches!(f.list("outdir"), Err(FilesError::OutsideRoot(_))));
        assert!(matches!(
            f.read("outdir/secret.txt"),
            Err(FilesError::OutsideRoot(_))
        ));
        let agent = AgentId::new("a");
        let leases = LeaseTable::new();
        assert!(matches!(
            f.write("outdir/new.txt", b"x", &agent, &leases, 0),
            Err(FilesError::OutsideRoot(_))
        ));
        assert!(matches!(
            f.write("link.txt", b"x", &agent, &leases, 0),
            Err(FilesError::OutsideRoot(_))
        ));
        assert_eq!(
            fs::read_to_string(d.join("secret.txt")).unwrap(),
            "top secret"
        );
        assert!(!d.join("new.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_the_root_work() {
        let (d, f) = setup();
        std::os::unix::fs::symlink(d.join("wt/src"), d.join("wt/alias")).unwrap();
        assert_eq!(f.read("alias/a.rs").unwrap(), b"fn a() {}");
    }

    #[test]
    fn writes_create_parents_and_respect_leases() {
        let (_d, f) = setup();
        let (a, b) = (AgentId::new("a"), AgentId::new("b"));
        let mut leases = LeaseTable::new();
        f.write("new/dir/x.txt", b"one", &a, &leases, 0).unwrap();
        assert_eq!(f.read("new/dir/x.txt").unwrap(), b"one");

        leases.acquire("src", &a, 1_000, 60_000).unwrap();
        // The holder may write; the other agent gets lease.held with the holder and expiry.
        f.write("src/a.rs", b"fn a2() {}", &a, &leases, 2_000)
            .unwrap();
        let e = f
            .write("src/a.rs", b"clobber", &b, &leases, 2_000)
            .unwrap_err();
        assert_eq!(
            e,
            FilesError::LeaseHeld {
                path: "src/a.rs".into(),
                holder: a.clone(),
                expires_at_ms: 61_000
            }
        );
        assert_eq!(e.to_protocol().code, ErrorCode::LeaseHeld);
        assert_eq!(f.read("src/a.rs").unwrap(), b"fn a2() {}");
        // After expiry the other agent may write.
        f.write("src/a.rs", b"b wins", &b, &leases, 61_000).unwrap();
        assert_eq!(f.read("src/a.rs").unwrap(), b"b wins");
    }

    #[test]
    fn watcher_batches_adds_changes_and_removals_and_skips_git() {
        let (d, f) = setup();
        let root = d.join("wt");
        let mut w = f.watch();
        assert!(w.poll().is_empty());

        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref").unwrap();
        fs::write(root.join("src/b.rs"), "new").unwrap();
        fs::write(root.join("src/a.rs"), "changed content").unwrap();
        fs::remove_file(root.join("README.md")).unwrap();
        assert_eq!(w.poll(), vec!["README.md", "src/a.rs", "src/b.rs"]);
        assert!(w.poll().is_empty());
    }
}
