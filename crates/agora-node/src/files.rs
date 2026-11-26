//! Files of one worktree: lazy listing, bounded reads, lease-checked writes, and change watching.
//!
//! Every path from a client is relative to the worktree root and is checked twice: lexically (no absolute
//! paths, no `..`) and after resolving symlinks (the real location must stay inside the root). Writes go
//! through the project's [`LeaseTable`]. Watching polls modification times, which needs no extra dependency
//! and behaves the same on every OS; a native watcher can replace it behind the same API.

use std::collections::BTreeMap;
use std::fs;
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use agora_core::secrets::is_secret_path;
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
    /// ADR-0080: the file is secret by its name (the requested path or the one it resolves to) and secret files are hidden.
    Secret(String),
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
            Self::Secret(p) => Error::new(ErrorCode::FilesSecret).with("path", p.as_str()),
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
    // A file where the path needs a folder (`README.md/x`, or listing `README.md`) names nothing there either (audit F18).
    if matches!(
        e.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    ) {
        FilesError::NotFound(path.to_string())
    } else if agora_platform::link_loop(e) {
        // A link to itself or a ring of links never leads anywhere: refused as a link that cannot be resolved is (issue #924).
        FilesError::OutsideRoot(path.to_string())
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
    /// ADR-0080: secret by its name (always computed; hiding it is the caller's rule).
    pub secret: bool,
}

/// Access to the files of one worktree.
#[derive(Debug, Clone)]
pub struct Files {
    root: PathBuf,
    max_read: u64,
    /// ADR-0080: the owner's `files.show_secret`. Off by default: a secret file is refused.
    show_secret: bool,
}

/// The points inside [`Files::write`] where a test makes the disk change under it.
#[allow(dead_code)] // the folder index is read by tests only
enum WriteStep {
    /// Before folder `n` of the path is opened.
    BeforeFolder(usize),
    /// After the temporary file is written, before it is renamed over the target.
    BeforeCommit,
    /// After the last folder is opened, before its real path is checked.
    AfterFolders,
}

/// The temporary file of a write: removed when dropped (an error or a panic) unless the rename happened, and only if the name still
/// is the file this write made.
struct PendingFile<'a> {
    dir: &'a agora_platform::ConfinedDir,
    name: std::ffi::OsString,
    file: Option<std::fs::File>,
}

impl PendingFile<'_> {
    fn committed(&mut self) {
        self.file = None;
        self.name = std::ffi::OsString::new();
    }
}

impl Drop for PendingFile<'_> {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = self.dir.remove_owned(&file, &self.name);
        }
    }
}

/// Why [`visit_inside`] gave nothing.
enum Inside {
    /// The folder's handle is not below the root (a link swapped in).
    Outside,
    Io(std::io::Error),
}

/// The entries of `dir`, read through its own handle once that handle is known to be below `root` (canonical).
fn visit_inside(
    root: &Path,
    dir: &Path,
    mut visitor: impl FnMut(&Path, agora_platform::DirItem) -> ControlFlow<()>,
) -> Result<(), Inside> {
    let handle = agora_platform::open_dir(dir).map_err(Inside::Io)?;
    let real = agora_platform::opened_path(&handle).map_err(Inside::Io)?;
    if !real.starts_with(root) {
        return Err(Inside::Outside);
    }
    agora_platform::visit_dir(&handle, |item| visitor(&real, item)).map_err(Inside::Io)
}

impl Files {
    /// `root` must exist; `max_read` is the largest file `read` returns, in bytes.
    pub fn open(root: &Path, max_read: u64) -> Result<Self, FilesError> {
        let root = fs::canonicalize(root).map_err(|e| io_err(&root.display().to_string(), &e))?;
        Ok(Self {
            root,
            max_read,
            show_secret: false,
        })
    }

    /// The folder this access is bound to: where `root` led when it was opened (canonical). Every listing and read is checked to be
    /// below it on the open handle, so a caller that must know the folder is still where it was compares this (audit F1011).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// ADR-0080: `rel` names a secret file by itself or by where it resolves to (links followed); false when it cannot be resolved.
    pub fn resolves_secret(&self, rel: &str) -> bool {
        // A file's own link is not followed (it is the link a diff shows), but its folder is.
        let Some(name) = Path::new(rel).file_name() else {
            return false;
        };
        let parent = Path::new(rel)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        match self.resolve(&parent) {
            Ok(dir) => self.is_secret(rel, &dir.join(name)),
            Err(_) => agora_core::secrets::is_secret_path(rel),
        }
    }

    /// ADR-0080: whether secret files are read (the owner's policy `files.show_secret = true`). The default is not.
    #[must_use]
    pub fn with_secret_shown(mut self, shown: bool) -> Self {
        self.show_secret = shown;
        self
    }

    /// `rel`, the path the client asked for, or `real`, where it really is, names a secret file (ADR-0080).
    fn is_secret(&self, rel: &str, real: &Path) -> bool {
        is_secret_path(rel)
            || real
                .strip_prefix(&self.root)
                .is_ok_and(|r| is_secret_path(&r.to_string_lossy()))
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
                // Something is there although it does not resolve: a link whose target is missing. A write would follow it
                // and create that target wherever it points, so it is refused rather than walked past.
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound
                        && fs::symlink_metadata(existing).is_ok() =>
                {
                    return Err(outside())
                }
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
    ///
    /// Listed through the folder's own handle after checking where that handle is, as [`Self::read`] does for a file: a folder
    /// swapped for a link after [`Self::resolve`]'s check would otherwise be listed wherever the link leads.
    pub fn list(&self, rel: &str) -> Result<Vec<Entry>, FilesError> {
        let dir = self.resolve(rel)?;
        let mut out = Vec::new();
        visit_inside(&self.root, &dir, |real, i| {
            let name = i.name.to_string_lossy().into_owned();
            out.push(Entry {
                // By the path asked for and by where a link leads (a folder `.ssh` makes what is in it secret).
                secret: self.is_secret(&format!("{rel}/{name}"), &real.join(&name)),
                name,
                is_dir: i.is_dir,
                size: if i.is_dir { 0 } else { i.len },
            });
            ControlFlow::Continue(())
        })
        .map_err(|e| match e {
            Inside::Outside => FilesError::OutsideRoot(rel.to_string()),
            Inside::Io(e) => io_err(rel, &e),
        })?;
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
        if !self.show_secret && self.is_secret(rel, &path) {
            return Err(FilesError::Secret(rel.to_string()));
        }
        // Without waiting: a FIFO swapped in for the file would block the open until something writes to it.
        let file = agora_platform::open_read(&path).map_err(|e| {
            // Something that is not a regular file and cannot be opened (a socket): not a file, as a folder or FIFO reads (audit F18).
            if fs::metadata(&path).is_ok_and(|m| !m.is_file()) {
                FilesError::NotFound(rel.to_string())
            } else {
                io_err(rel, &e)
            }
        })?;
        let real = agora_platform::opened_path(&file).map_err(|e| io_err(rel, &e))?;
        if !real.starts_with(&self.root) {
            return Err(FilesError::OutsideRoot(rel.to_string()));
        }
        let meta = file.metadata().map_err(|e| io_err(rel, &e))?;
        // Where the open file really is, once more: a name swapped for a link after the check above.
        if !self.show_secret && self.is_secret(rel, &real) {
            return Err(FilesError::Secret(rel.to_string()));
        }
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

    /// Write a file on behalf of `agent`, unless another agent holds a live lease covering it. Missing parent folders are created.
    ///
    /// Dormant (ADR-0080: `files.write` is disabled and no request path calls this), and built so that exposing it later is safe:
    ///
    /// - the root is opened once and every folder is then opened relative to the one before it, never following a link, so a
    ///   folder swapped for a link between steps is refused, or the write lands in the folder that was verified, never elsewhere;
    /// - the file is written to a temporary name in that verified folder and renamed over the target, so a failure leaves nothing
    ///   behind (a guard removes the temporary file even on a panic, and only if the name still is the file this call made); a link, folder,
    ///   FIFO or device at the target is refused (a folder, FIFO or device reads as `files.not_found`, as a read of one does: the
    ///   protocol has no code for "not a regular file"); a link swapped in for the target after the check is replaced as itself;
    /// - the secret-name policy of reads applies to the requested path, to each folder prefix, and to where the folder really is;
    /// - `max_read` is also the largest write, checked before anything is created.
    ///
    /// A folder inside the worktree that is itself a link is refused for writes (reads follow it): a write never goes through one.
    /// A new file is `0o600`; a replaced file keeps its owner permission bits and read/execute for others, never write for group or
    /// other. Owner, group and extended attributes are not preserved. Folders created for the path stay if a later step fails.
    pub fn write(
        &self,
        rel: &str,
        bytes: &[u8],
        agent: &AgentId,
        leases: &LeaseTable,
        now_ms: u64,
    ) -> Result<(), FilesError> {
        self.write_with(rel, bytes, agent, leases, now_ms, &mut |_| {})
    }

    fn write_with(
        &self,
        rel: &str,
        bytes: &[u8],
        agent: &AgentId,
        leases: &LeaseTable,
        now_ms: u64,
        hook: &mut dyn FnMut(WriteStep),
    ) -> Result<(), FilesError> {
        let outside = || FilesError::OutsideRoot(rel.to_string());
        let mut names = Vec::new();
        for c in Path::new(rel).components() {
            match c {
                Component::Normal(p) if agora_platform::valid_component(p) => names.push(p),
                Component::CurDir => {}
                // `..`, `/x`, `C:\x`, and names a platform reads two ways: never allowed.
                _ => return Err(outside()),
            }
        }
        let Some((file_name, folders)) = names.split_last() else {
            return Err(outside());
        };
        if let Some(l) = leases.blocking_lease(rel, agent, now_ms) {
            return Err(FilesError::LeaseHeld {
                path: rel.to_string(),
                holder: l.holder.clone(),
                expires_at_ms: l.expires_at_ms,
            });
        }
        // Names only, before the disk is touched: the path as asked, and each prefix of it (a folder named like a secret file).
        if !self.show_secret {
            let mut prefix = String::new();
            let mut secret = is_secret_path(rel);
            for n in &names {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(&n.to_string_lossy());
                secret |= is_secret_path(&prefix);
            }
            if secret {
                return Err(FilesError::Secret(rel.to_string()));
            }
        }
        if bytes.len() as u64 > self.max_read {
            return Err(FilesError::TooLarge {
                path: rel.to_string(),
                limit: self.max_read,
            });
        }

        let confined = |e: std::io::Error| {
            if agora_platform::not_confined(&e) {
                outside()
            } else {
                io_err(rel, &e)
            }
        };
        let mut dir = agora_platform::ConfinedDir::open_root(&self.root).map_err(confined)?;
        if dir.real_path().map_err(|e| io_err(rel, &e))? != self.root {
            return Err(outside());
        }
        for (i, name) in folders.iter().enumerate() {
            hook(WriteStep::BeforeFolder(i));
            dir = dir.subdir(name, true).map_err(confined)?;
        }
        hook(WriteStep::AfterFolders);
        // Where the verified folder really is (the kernel's spelling), with the secret policy once more.
        let real = dir.real_path().map_err(|e| io_err(rel, &e))?;
        if !real.starts_with(&self.root) {
            return Err(outside());
        }
        if !self.show_secret && self.is_secret(rel, &real.join(file_name)) {
            return Err(FilesError::Secret(rel.to_string()));
        }

        let mode = match dir.entry(file_name).map_err(confined)? {
            None => 0o600,
            // Group and other never get write, whatever the old file had (or the umask allows).
            Some(e) if e.kind == agora_platform::EntryKind::File => e.mode & 0o755,
            Some(e) if e.kind == agora_platform::EntryKind::Link => return Err(outside()),
            Some(_) => return Err(FilesError::NotFound(rel.to_string())),
        };

        static TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut attempt = 0;
        let mut pending = loop {
            let temp = std::ffi::OsString::from(format!(
                ".agora-write-{}-{}",
                std::process::id(),
                TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            match dir.create_new(&temp, mode) {
                Ok(file) => {
                    break PendingFile {
                        dir: &dir,
                        name: temp,
                        file: Some(file),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 8 => {
                    attempt += 1;
                }
                Err(e) => return Err(confined(e)),
            }
        };
        let result = (|| {
            use std::io::Write;
            let file = pending.file.as_mut().ok_or(std::io::ErrorKind::Other)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            hook(WriteStep::BeforeCommit);
            dir.rename_over(file, &pending.name, file_name)
        })();
        result.map_err(confined)?;
        pending.committed();
        Ok(())
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
            if out.len() >= MAX_ENTRIES {
                break;
            }
            // Through the folder's own handle, checked to be below the root: a folder found here can be a link by the time it
            // is opened, and the walk would go on wherever it leads.
            // Stop the handle's enumeration at the budget; collecting a directory first defeats this bound.
            // As before, a failed directory scan does not stop watching other directories.
            let _ = visit_inside(&self.root, &dir, |_, item| {
                let path = dir.join(&item.name);
                if item.is_dir {
                    if !SKIP_DIRS.iter().any(|s| item.name == *s) {
                        stack.push(path);
                    }
                } else {
                    let Ok(rel) = path.strip_prefix(&self.root).map(Path::to_path_buf) else {
                        return ControlFlow::Continue(());
                    };
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    out.insert(rel, (item.modified, item.len));
                }
                if out.len() >= MAX_ENTRIES {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
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
    fn a_path_through_a_file_or_to_a_socket_is_not_found() {
        // Audit F18: these answered `Io("Not a directory")` / `Io("No such device or address")`, shown as internal.
        let (d, f) = setup();
        assert_eq!(
            f.read_file("README.md/x"),
            Err(FilesError::NotFound("README.md/x".into()))
        );
        assert_eq!(
            f.list("README.md"),
            Err(FilesError::NotFound("README.md".into()))
        );
        #[cfg(unix)]
        {
            let _sock = std::os::unix::net::UnixListener::bind(d.join("wt/sock")).unwrap();
            assert!(f.list("").unwrap().iter().any(|e| e.name == "sock"));
            assert_eq!(
                f.read_file("sock"),
                Err(FilesError::NotFound("sock".into()))
            );
        }
        #[cfg(not(unix))]
        let _ = d;
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

    /// Flips `root/d` between a folder of its own and a link to `outside` until `stop`, and answers how many times `d` became the
    /// link. The link is made once (a junction on Windows, which needs no privilege) and the flipping is renames only, so it is fast
    /// on every system. A link that cannot be made fails the test: a race test that skips itself tests nothing.
    fn flip_folder(
        root: &Path,
        outside: &Path,
        stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> std::thread::JoinHandle<u64> {
        let (name, link, spare) = (root.join("d"), root.join("d.link"), root.join("d.spare"));
        fs::create_dir_all(&name).unwrap();
        fs::write(name.join("MINE"), "x").unwrap();
        agora_platform::link_dir(outside, &link).expect("a folder link (a junction on Windows)");
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut flips = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                // A step that fails (the folder is open on Windows) leaves the names as they were; the next round puts them back.
                let _ = fs::rename(&name, &spare);
                if fs::rename(&link, &name).is_ok() {
                    flips += 1;
                }
                let _ = fs::rename(&name, &link);
                let _ = fs::rename(&spare, &name);
            }
            flips
        })
    }

    #[test]
    fn a_folder_swapped_for_a_link_after_the_check_is_not_listed() {
        let (d, f) = setup();
        let outside = d.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("OUTSIDE-NAME"), "x").unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flipper = flip_folder(&d.join("wt"), &outside, &stop);
        let mut leaked = 0;
        for _ in 0..20_000 {
            if f.list("d")
                .is_ok_and(|e| e.iter().any(|e| e.name == "OUTSIDE-NAME"))
            {
                leaked += 1;
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let flips = flipper.join().unwrap();
        assert!(flips > 0, "the folder was never swapped for the link");
        assert_eq!(leaked, 0, "files.list showed a folder outside the worktree");
    }

    #[test]
    fn the_watcher_never_walks_into_a_folder_swapped_for_a_link() {
        let (d, f) = setup();
        let outside = d.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("OUTSIDE-NAME"), "x").unwrap();
        let mut w = f.watch();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flipper = flip_folder(&d.join("wt"), &outside, &stop);
        let mut leaked = 0;
        for _ in 0..3_000 {
            if w.poll().iter().any(|p| p.ends_with("OUTSIDE-NAME")) {
                leaked += 1;
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let flips = flipper.join().unwrap();
        assert!(flips > 0, "the folder was never swapped for the link");
        assert_eq!(
            leaked, 0,
            "the watcher reported a file outside the worktree"
        );
    }

    #[test]
    fn a_file_swapped_for_a_link_after_the_check_is_not_read() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let (d, f) = setup();
        let root = d.join("wt");
        // The folder above the file swapped for a link, which every system can make (a junction on Windows).
        let outside = d.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("notes.md"), "top secret").unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flipper = flip_folder(&root, &outside, &stop);
        let mut leaked = 0;
        for _ in 0..20_000 {
            if f.read("d/notes.md").is_ok_and(|data| data == b"top secret") {
                leaked += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        let flips = flipper.join().unwrap();
        assert!(flips > 0, "the folder was never swapped for the link");
        assert_eq!(
            leaked, 0,
            "the node read the secret through a swapped folder"
        );
        // The file itself swapped for a link: only where a file link can be made (not on Windows without the privilege).
        if agora_platform::symlink(&d.join("secret.txt"), &root.join("probe")).is_err() {
            return;
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

    #[test]
    fn a_write_never_follows_a_link_whose_target_is_missing() {
        let (d, f) = setup();
        let root = d.join("wt");
        // The agent plants a link to a file that does not exist yet, outside its worktree.
        if agora_platform::symlink(&d.join("planted.txt"), &root.join("dangling")).is_err() {
            return; // no symbolic links here (Windows without the privilege)
        }
        let (agent, leases) = (AgentId::new("a"), LeaseTable::new());
        assert!(matches!(
            f.write("dangling", b"x", &agent, &leases, 0),
            Err(FilesError::OutsideRoot(_))
        ));
        assert!(
            !d.join("planted.txt").exists(),
            "the node created a file outside the worktree"
        );
        // New files in new folders are unchanged.
        f.write("fresh/dir/y.txt", b"y", &agent, &leases, 0)
            .unwrap();
        assert_eq!(f.read("fresh/dir/y.txt").unwrap(), b"y");
    }

    #[test]
    fn a_link_that_never_resolves_is_outside_the_root_not_an_internal_error() {
        // Issue #924: a link to itself, or a ring of links, never leads anywhere; it answers as a link that cannot be resolved does.
        let (d, f) = setup();
        let root = d.join("wt");
        if agora_platform::symlink(Path::new("self"), &root.join("self")).is_err() {
            return; // no symbolic links here (Windows without the privilege)
        }
        agora_platform::symlink(Path::new("ring-b"), &root.join("ring-a")).unwrap();
        agora_platform::symlink(Path::new("ring-a"), &root.join("ring-b")).unwrap();
        let (agent, leases) = (AgentId::new("a"), LeaseTable::new());
        for rel in ["self", "ring-a", "self/x.txt", "ring-b/x.txt"] {
            let outside = |r: Result<(), FilesError>| matches!(r, Err(FilesError::OutsideRoot(p)) if p == rel);
            assert!(
                outside(f.read(rel).map(drop)),
                "read {rel}: {:?}",
                f.read(rel)
            );
            assert!(
                outside(f.list(rel).map(drop)),
                "list {rel}: {:?}",
                f.list(rel)
            );
            let wrote = f.write(rel, b"x", &agent, &leases, 0);
            assert!(outside(wrote.clone()), "write {rel}: {wrote:?}");
        }
        // The links themselves are untouched, and a diff still shows a link as its target.
        assert_eq!(fs::read_link(root.join("self")).unwrap(), Path::new("self"));
        assert_eq!(
            f.untracked("self").unwrap(),
            Untracked::Link(b"self".to_vec())
        );
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

    fn only_files_under(dir: &Path) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let p = e.path();
                let ft = e.file_type().unwrap();
                if ft.is_dir() {
                    walk(&p, base, out);
                } else {
                    out.push(
                        p.strip_prefix(base)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    fn write_plain(f: &Files, rel: &str, bytes: &[u8]) -> Result<(), FilesError> {
        f.write(rel, bytes, &AgentId::new("a"), &LeaseTable::new(), 0)
    }

    #[test]
    fn a_write_replaces_atomically_and_leaves_no_temporary_file() {
        let (d, f) = setup();
        write_plain(&f, "src/a.rs", b"new body").unwrap();
        write_plain(&f, "./src/./b.rs", b"b").unwrap();
        assert_eq!(f.read("src/a.rs").unwrap(), b"new body");
        assert_eq!(
            only_files_under(&d.join("wt")),
            vec!["README.md", "src/a.rs", "src/b.rs"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_file_keeps_its_permission_bits() {
        use std::os::unix::fs::PermissionsExt;
        let (d, f) = setup();
        let p = d.join("wt/src/a.rs");
        fs::set_permissions(&p, fs::Permissions::from_mode(0o750)).unwrap();
        write_plain(&f, "src/a.rs", b"x").unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }

    #[test]
    fn dot_dot_absolute_and_empty_paths_are_outside() {
        let (d, f) = setup();
        for rel in [
            "../x.txt",
            "src/../../x.txt",
            "/etc/x",
            "",
            ".",
            "src/..",
            "a\\..\\b",
        ] {
            assert!(
                matches!(write_plain(&f, rel, b"x"), Err(FilesError::OutsideRoot(_))),
                "{rel:?}"
            );
        }
        assert!(!d.join("x.txt").exists());
    }

    #[test]
    fn secret_names_are_refused_by_every_component_and_never_created() {
        let (d, f) = setup();
        for rel in [
            ".env",
            "src/.ENV",
            ".ssh/config",
            "a/.aws/credentials",
            "keys/server.pem",
            ".docker/config.json",
            "src/id_rsa/inner.txt",
        ] {
            assert!(
                matches!(write_plain(&f, rel, b"x"), Err(FilesError::Secret(_))),
                "{rel}"
            );
        }
        assert_eq!(
            only_files_under(&d.join("wt")),
            vec!["README.md", "src/a.rs"]
        );
        assert!(!d.join("wt/.ssh").exists() && !d.join("wt/a").exists());
        // The owner's policy that shows secret files applies to writes as it does to reads.
        let shown = Files::open(&d.join("wt"), 1024)
            .unwrap()
            .with_secret_shown(true);
        write_plain(&shown, ".env", b"K=1").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_really_is_secret_is_refused_by_the_real_path_check() {
        // The names asked for pass the first check; the folder is moved to a secret name after it was opened, so only the check of
        // where the handle really is can catch it.
        let (d, f) = setup();
        let root = d.join("wt");
        fs::create_dir_all(root.join("sub")).unwrap();
        let e = f
            .write_with(
                "sub/x.txt",
                b"x",
                &AgentId::new("a"),
                &LeaseTable::new(),
                0,
                &mut |step| {
                    if matches!(step, WriteStep::AfterFolders) {
                        fs::rename(root.join("sub"), root.join(".ssh")).unwrap();
                    }
                },
            )
            .unwrap_err();
        assert!(matches!(e, FilesError::Secret(_)), "{e:?}");
        assert!(only_files_under(&root.join(".ssh")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_temporary_name_swapped_for_a_link_is_neither_renamed_nor_removed() {
        let (d, f) = setup();
        let root = d.join("wt");
        let outside = d.join("secret.txt");
        let e = f
            .write_with(
                "src/a.rs",
                b"mine",
                &AgentId::new("a"),
                &LeaseTable::new(),
                0,
                &mut |step| {
                    if matches!(step, WriteStep::BeforeCommit) {
                        let temp = fs::read_dir(root.join("src"))
                            .unwrap()
                            .map(|e| e.unwrap().path())
                            .find(|p| {
                                p.file_name()
                                    .unwrap()
                                    .to_string_lossy()
                                    .starts_with(".agora-write-")
                            })
                            .unwrap();
                        fs::remove_file(&temp).unwrap();
                        std::os::unix::fs::symlink(&outside, &temp).unwrap();
                    }
                },
            )
            .unwrap_err();
        assert!(matches!(e, FilesError::OutsideRoot(_)), "{e:?}");
        assert_eq!(fs::read(&outside).unwrap(), b"top secret");
        assert_eq!(fs::read(root.join("src/a.rs")).unwrap(), b"fn a() {}");
        // The planted link is not ours: it stays.
        let left: Vec<_> = fs::read_dir(root.join("src"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert!(left
            .iter()
            .any(|p| fs::symlink_metadata(p).unwrap().file_type().is_symlink()));
    }

    #[test]
    fn a_panic_in_the_middle_removes_the_temporary_file() {
        let (d, f) = setup();
        let root = d.join("wt");
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = f.write_with(
                "src/a.rs",
                b"x",
                &AgentId::new("a"),
                &LeaseTable::new(),
                0,
                &mut |step| {
                    if matches!(step, WriteStep::BeforeCommit) {
                        panic!("injected");
                    }
                },
            );
        }));
        assert!(r.is_err());
        assert_eq!(only_files_under(&root), vec!["README.md", "src/a.rs"]);
        assert_eq!(fs::read(root.join("src/a.rs")).unwrap(), b"fn a() {}");
    }

    #[cfg(unix)]
    #[test]
    fn modes_never_give_group_or_other_write() {
        use std::os::unix::fs::PermissionsExt;
        let (d, f) = setup();
        let p = d.join("wt/src/a.rs");
        fs::set_permissions(&p, fs::Permissions::from_mode(0o666)).unwrap();
        write_plain(&f, "src/a.rs", b"x").unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o644
        );
        write_plain(&f, "src/new.rs", b"x").unwrap();
        assert_eq!(
            fs::metadata(d.join("wt/src/new.rs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn a_payload_over_the_limit_is_refused_before_anything_is_created() {
        let (d, f) = setup();
        let big = vec![7u8; 1025];
        let e = write_plain(&f, "new/deep/big.bin", &big).unwrap_err();
        assert_eq!(
            e,
            FilesError::TooLarge {
                path: "new/deep/big.bin".into(),
                limit: 1024
            }
        );
        assert!(!d.join("wt/new").exists());
        write_plain(&f, "src/a.rs", &vec![1u8; 1024]).unwrap();
    }

    #[test]
    fn a_lease_held_by_another_agent_blocks_before_the_disk_changes() {
        let (d, f) = setup();
        let (a, b) = (AgentId::new("a"), AgentId::new("b"));
        let mut leases = LeaseTable::new();
        leases.acquire("fresh", &a, 1_000, 60_000).unwrap();
        let e = f
            .write("fresh/deep/x.txt", b"x", &b, &leases, 2_000)
            .unwrap_err();
        assert_eq!(e.to_protocol().code, ErrorCode::LeaseHeld);
        assert!(!d.join("wt/fresh").exists());
        f.write("fresh/deep/x.txt", b"x", &a, &leases, 2_000)
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_as_the_final_component_is_replaced_never_followed() {
        let (d, f) = setup();
        let outside = d.join("secret.txt");
        std::os::unix::fs::symlink(&outside, d.join("wt/link.txt")).unwrap();
        // Refused, and the file it points at is untouched.
        assert!(matches!(
            write_plain(&f, "link.txt", b"pwned"),
            Err(FilesError::OutsideRoot(_))
        ));
        assert_eq!(fs::read(&outside).unwrap(), b"top secret");
        // A dangling link is refused the same way, and nothing is created at its target.
        std::os::unix::fs::symlink(d.join("nowhere.txt"), d.join("wt/dangling.txt")).unwrap();
        assert!(matches!(
            write_plain(&f, "dangling.txt", b"x"),
            Err(FilesError::OutsideRoot(_))
        ));
        assert!(!d.join("nowhere.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_as_a_folder_component_is_refused_even_inside_the_root() {
        let (d, f) = setup();
        std::os::unix::fs::symlink(d.join("wt/src"), d.join("wt/alias")).unwrap();
        std::os::unix::fs::symlink(&d, d.join("wt/out")).unwrap();
        for rel in ["alias/x.rs", "out/x.rs", "out/wt/x.rs"] {
            assert!(
                matches!(write_plain(&f, rel, b"x"), Err(FilesError::OutsideRoot(_))),
                "{rel}"
            );
        }
        assert!(!d.join("x.rs").exists() && !d.join("wt/src/x.rs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_a_folder_or_a_device_at_the_target_is_refused() {
        let (d, f) = setup();
        let fifo = d.join("wt/pipe");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        fs::create_dir(d.join("wt/adir")).unwrap();
        for rel in ["pipe", "adir", "src"] {
            assert!(
                matches!(write_plain(&f, rel, b"x"), Err(FilesError::NotFound(_))),
                "{rel}"
            );
        }
        // Still a FIFO, and no temporary file beside it.
        assert!(std::os::unix::fs::FileTypeExt::is_fifo(
            &fs::symlink_metadata(&fifo).unwrap().file_type()
        ));
        assert_eq!(
            only_files_under(&d.join("wt")),
            vec!["README.md", "pipe", "src/a.rs"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_swapped_for_a_link_before_it_is_opened_is_refused() {
        let (d, f) = setup();
        let root = d.join("wt");
        fs::create_dir_all(root.join("sub/inner")).unwrap();
        let mut swapped = false;
        let e = f
            .write_with(
                "sub/inner/x.txt",
                b"x",
                &AgentId::new("a"),
                &LeaseTable::new(),
                0,
                &mut |step| {
                    if matches!(step, WriteStep::BeforeFolder(1)) && !swapped {
                        swapped = true;
                        fs::remove_dir_all(root.join("sub/inner")).unwrap();
                        std::os::unix::fs::symlink(&d, root.join("sub/inner")).unwrap();
                    }
                },
            )
            .unwrap_err();
        assert!(swapped);
        assert!(matches!(e, FilesError::OutsideRoot(_)));
        assert!(!d.join("x.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_swapped_for_a_link_after_it_was_opened_cannot_redirect_the_write() {
        let (d, f) = setup();
        let root = d.join("wt");
        fs::create_dir_all(root.join("sub")).unwrap();
        let elsewhere = d.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        let mut swapped = false;
        f.write_with(
            "sub/x.txt",
            b"inside",
            &AgentId::new("a"),
            &LeaseTable::new(),
            0,
            &mut |step| {
                if matches!(step, WriteStep::BeforeCommit) {
                    swapped = true;
                    // The verified folder is moved away and a link to another place takes its name.
                    fs::rename(root.join("sub"), d.join("moved")).unwrap();
                    std::os::unix::fs::symlink(&elsewhere, root.join("sub")).unwrap();
                }
            },
        )
        .unwrap();
        assert!(swapped);
        // The file went into the folder that was verified, and nothing reached the link's target.
        assert_eq!(fs::read(d.join("moved/x.txt")).unwrap(), b"inside");
        assert!(fs::read_dir(&elsewhere).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_swapped_in_for_the_target_before_the_commit_is_replaced_not_followed() {
        let (d, f) = setup();
        let root = d.join("wt");
        let outside = d.join("secret.txt");
        f.write_with(
            "src/a.rs",
            b"mine",
            &AgentId::new("a"),
            &LeaseTable::new(),
            0,
            &mut |step| {
                if matches!(step, WriteStep::BeforeCommit) {
                    fs::remove_file(root.join("src/a.rs")).unwrap();
                    std::os::unix::fs::symlink(&outside, root.join("src/a.rs")).unwrap();
                }
            },
        )
        .unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"top secret");
        assert!(!fs::symlink_metadata(root.join("src/a.rs"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(root.join("src/a.rs")).unwrap(), b"mine");
    }

    #[cfg(unix)]
    #[test]
    fn an_error_in_the_middle_leaves_the_old_file_and_no_temporary_file() {
        let (d, f) = setup();
        let root = d.join("wt");
        // The target becomes a folder just before the rename: the rename fails after the data was written.
        let e = f
            .write_with(
                "src/a.rs",
                b"new",
                &AgentId::new("a"),
                &LeaseTable::new(),
                0,
                &mut |step| {
                    if matches!(step, WriteStep::BeforeCommit) {
                        fs::remove_file(root.join("src/a.rs")).unwrap();
                        fs::create_dir(root.join("src/a.rs")).unwrap();
                    }
                },
            )
            .unwrap_err();
        assert!(matches!(e, FilesError::Io(_)), "{e:?}");
        assert_eq!(only_files_under(&root), vec!["README.md"]);
        assert!(root.join("src/a.rs").is_dir());
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

    fn secret_setup() -> (PathBuf, PathBuf) {
        let d = tmp();
        let root = d.join("wt");
        fs::create_dir_all(root.join(".ssh")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(".env"), "TOKEN=abc\n").unwrap();
        fs::write(root.join(".env.example"), "TOKEN=\n").unwrap();
        fs::write(root.join("id_rsa"), "key").unwrap();
        fs::write(root.join("id_rsa.pub"), "pub").unwrap();
        fs::write(root.join(".ssh/config"), "Host x").unwrap();
        fs::write(root.join("src/a.rs"), "fn a() {}").unwrap();
        (d, root)
    }

    #[test]
    fn a_listing_marks_secret_entries_and_does_not_hide_them() {
        let (_d, root) = secret_setup();
        let f = Files::open(&root, 1024).unwrap();
        let marks: Vec<(String, bool)> = f
            .list("")
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.secret))
            .collect();
        for (name, secret) in [
            (".env", true),
            ("id_rsa", true),
            (".env.example", false),
            ("id_rsa.pub", false),
            ("src", false),
            (".ssh", false),
        ] {
            assert!(
                marks.contains(&(name.to_string(), secret)),
                "{name}: {marks:?}"
            );
        }
        // What is inside a secret folder is secret, whatever its own name.
        assert!(f.list(".ssh").unwrap().iter().all(|e| e.secret));
        assert!(f.list("src").unwrap().iter().all(|e| !e.secret));
    }

    #[test]
    fn reading_a_secret_file_is_refused_and_an_ordinary_one_is_not() {
        let (_d, root) = secret_setup();
        let f = Files::open(&root, 1024).unwrap();
        for p in [".env", "id_rsa", ".ssh/config", "./.ENV"] {
            assert_eq!(f.read(p), Err(FilesError::Secret(p.to_string())), "{p}");
        }
        assert_eq!(f.read(".env.example").unwrap(), b"TOKEN=\n");
        assert_eq!(f.read("id_rsa.pub").unwrap(), b"pub");
        assert_eq!(f.read("src/a.rs").unwrap(), b"fn a() {}");
        let e = FilesError::Secret(".env".into()).to_protocol();
        assert_eq!(e.to_string(), FilesError::Secret(".env".into()).to_string());
    }

    #[test]
    fn the_owners_switch_allows_reading_secret_files() {
        let (_d, root) = secret_setup();
        let f = Files::open(&root, 1024).unwrap().with_secret_shown(true);
        assert_eq!(f.read(".env").unwrap(), b"TOKEN=abc\n");
        assert_eq!(f.read(".ssh/config").unwrap(), b"Host x");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_secret_file_or_folder_is_refused_by_where_it_leads() {
        let (_d, root) = secret_setup();
        std::os::unix::fs::symlink(root.join(".env"), root.join("notes.txt")).unwrap();
        std::os::unix::fs::symlink(root.join(".ssh"), root.join("keys")).unwrap();
        let f = Files::open(&root, 1024).unwrap();
        assert_eq!(
            f.read("notes.txt"),
            Err(FilesError::Secret("notes.txt".into()))
        );
        assert_eq!(
            f.read("keys/config"),
            Err(FilesError::Secret("keys/config".into()))
        );
        assert!(f.list("keys").unwrap().iter().all(|e| e.secret));
        // Negative control: a link to an ordinary file reads.
        std::os::unix::fs::symlink(root.join("src/a.rs"), root.join("ok.txt")).unwrap();
        assert_eq!(f.read("ok.txt").unwrap(), b"fn a() {}");
        assert_eq!(
            f.with_secret_shown(true).read("notes.txt").unwrap(),
            b"TOKEN=abc\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_checks_secret_against_opened_real_dir_under_toctou_swap() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("agora-files-toctou-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".ssh")).unwrap();
        std::fs::create_dir_all(dir.join("pub_plain")).unwrap();
        std::fs::write(dir.join(".ssh/custom_key"), b"SECRET").unwrap();
        std::fs::write(dir.join("pub_plain/readme.txt"), b"hello").unwrap();
        let pub_path = dir.join("pub");
        std::fs::create_dir_all(&pub_path).unwrap();

        let files = Files::open(&dir, 1024).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_w = Arc::clone(&stop);
        let pub_w = pub_path.clone();
        let ssh_dir = dir.join(".ssh");
        let plain_dir = dir.join("pub_plain");
        let swapper = std::thread::spawn(move || {
            let mut toggle = false;
            while !stop_w.load(Ordering::Relaxed) {
                let _ = std::fs::remove_dir(&pub_w);
                let _ = std::fs::remove_file(&pub_w);
                let target = if toggle { &ssh_dir } else { &plain_dir };
                let _ = std::os::unix::fs::symlink(target, &pub_w);
                toggle = !toggle;
            }
        });

        for _ in 0..2000 {
            if let Ok(entries) = files.list("pub") {
                assert!(
                    !entries.iter().any(|e| e.name == "custom_key" && !e.secret),
                    "leaked secret=false entry: {entries:?}"
                );
            }
        }
        stop.store(true, Ordering::Relaxed);
        swapper.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
