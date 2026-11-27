//! ADR-0064: the workspaces (worktrees) this node made for its agents, and their clean-up.
//!
//! **What is recorded.** Every worktree the node makes for an agent (`git::create_start_worktree`, through the supervisor) is
//! recorded the moment git made it, before any agent runs there, in one owner-only file per worktree in `<data>/workspaces/`: its
//! place, the checkout and branch it came from, and its Git folders and link files as git wrote them ([`git::MadeLinks`]). Nothing
//! else is ever a candidate for removal: a selected folder (ADR-0052), a worktree a person made, or one from before this record existed
//! is never touched. A worktree whose place or links do not check out when it is made is not recorded.
//!
//! **When one goes.** Only when all hold ([`decide`]): no agent runs (or waits to be resumed) in it or in a folder below it; the
//! worktree is still a real folder directly in the node's worktree folder, at its recorded place; its `.git`, `commondir` and
//! `gitdir` still hold what git wrote ([`git::MadeLinks::still_hold`]), so every git answer comes from the recorded repository; git
//! says it has no uncommitted change (untracked files included), no checked-out submodule and no commit that no other branch or
//! remote has ([`git::keeps_work`]); every file git ignores there is regenerable build output or a cache ([`needed_ignored`], the
//! maintainer's rule: "if git ignores it and it is unimportant, delete it; if it is needed, keep it"); and, for the node's own pass, the
//! grace period passed since its agent ended. Removal is
//! `git::remove_worktree` without force, which checks all of that again; the branch is never deleted. Anything else leaves it in
//! place as "left behind" with its reason ([`Reason`]). Every removal is written to `workspaces.log`.
//!
//! **Two removers.** The node's pass and `agora workspaces clean` may run at once. A workspace is claimed before it is checked, by
//! renaming its record (`rename_no_replace`, never replacing): only one process holds a claim, and a start or resume into a claimed
//! folder is refused ([`Workspaces::removing`], checked by the supervisor under its lock). Whether the folder is in use is asked again
//! after the claim, so a start that came first is seen. A claim is stamped with when it was made and by which process; it is given
//! back by another process only when it is over an hour old and that process is gone (a crash), never while its holder runs.
//!
//! **Build output.** A left-behind worktree whose grace period passed has its `target/` (beside a `Cargo.toml`) and `node_modules/`
//! (beside a `package.json`) removed, once: only a real folder at the top of the worktree that git says is ignored and holds nothing
//! tracked. Nothing else is ever guessed.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::git::{self, Keeps, MadeLinks};

/// The registry's folder in the node's data folder.
pub const DIR: &str = "workspaces";
/// Every removal, one line each (in [`DIR`]).
pub const LOG_FILE: &str = "workspaces.log";
const RECORD_EXT: &str = "json";
const CLAIM_EXT: &str = "claim";
/// The most a record file is read; anything longer is not one the node wrote.
const RECORD_MAX: u64 = 64 << 10;
/// The most records read; more are not ones this node wrote.
const MAX_RECORDS: usize = 4096;
/// A claim older than this was left by a process that ended mid-way (a crash): it is given back.
const CLAIM_STALE_MS: u64 = 60 * 60 * 1000;
const HOUR_MS: u64 = 60 * 60 * 1000;
const GB: u64 = 1 << 30;
/// Build output removed from a left-behind worktree, each only beside its project file (ADR-0064 decision 4's defaults).
const BUILD_OUTPUT: [(&str, &str); 2] =
    [("Cargo.toml", "target"), ("package.json", "node_modules")];
/// Ignored folders that are regenerable build output or caches (the maintainer's rule, 2026-11-27): a worktree whose ignored paths are
/// all these, [`DISPOSABLE_FILES`] and `target/` beside a `Cargo.toml` may be removed. Any other ignored path (`.env*`, `*.db`,
/// `*.sqlite*`, keys and certificates, notes) is needed: the worktree stays as left behind with reason `ignored`.
pub const DISPOSABLE_DIRS: [&str; 15] = [
    "node_modules",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".turbo",
    ".cache",
    ".parcel-cache",
    "coverage",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".gradle",
];
/// Ignored files that are regenerable (see [`DISPOSABLE_DIRS`]): a name, or `*` and a suffix.
pub const DISPOSABLE_FILES: [&str; 3] = ["*.pyc", "*.log", ".DS_Store"];

/// Whether `entry` (a path git ignores in `work`, as [`git::ignored_paths`] lists it: a folder ends in `/`) is build output or a cache.
fn disposable(work: &Path, entry: &str) -> bool {
    if let Some(dir) = entry.strip_suffix('/') {
        let last = dir.rsplit('/').next().unwrap_or(dir);
        if last == "target" {
            // Only a Rust build folder: beside its `Cargo.toml`.
            let beside = work.join(dir).with_file_name("Cargo.toml");
            return std::fs::symlink_metadata(beside).is_ok_and(|m| m.is_file());
        }
        return DISPOSABLE_DIRS.contains(&last);
    }
    let base = entry.rsplit('/').next().unwrap_or(entry);
    DISPOSABLE_FILES.iter().any(|f| match f.strip_prefix('*') {
        Some(suffix) => base.len() > suffix.len() && base.ends_with(suffix),
        None => base == *f,
    })
}

/// The ignored paths in `work` that are not build output or caches ([`disposable`]): what removing it would lose.
pub fn needed_ignored(work: &Path, entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| !disposable(work, e))
        .cloned()
        .collect()
}

/// The owner's settings (policy file `workspaces.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// `workspaces.clean = auto | off`: the node's own pass removes what it may. `off` (or any other word) only lists.
    pub auto_clean: bool,
    /// `workspaces.grace_hours`: how long after its agent ended a workspace is kept by the node's pass (default 24).
    pub grace_ms: u64,
    /// `workspaces.max_gb`: the most the node's worktrees may hold together before it refuses to make another (default 50; 0: no cap).
    pub max_bytes: u64,
    /// `workspaces.min_free_gb`: below this much free disk the node refuses to make a worktree (default 3; 0: never refuses).
    pub min_free_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_clean: true,
            grace_ms: 24 * HOUR_MS,
            max_bytes: 50 * GB,
            min_free_bytes: 3 * GB,
        }
    }
}

impl Settings {
    /// Takes one `workspaces.*` line of the policy file. `None`: not one of these keys; `Some(false)`: one of them with a value it does
    /// not understand, which keeps the default.
    pub fn take(&mut self, key: &str, value: &str) -> Option<bool> {
        let number = || value.trim().parse::<u64>().ok();
        match key {
            "workspaces.clean" => {
                self.auto_clean = value.trim() == "auto";
                Some(matches!(value.trim(), "auto" | "off"))
            }
            "workspaces.grace_hours" => Some(number().is_some_and(|h| {
                self.grace_ms = h.saturating_mul(HOUR_MS);
                true
            })),
            "workspaces.max_gb" => Some(number().is_some_and(|g| {
                self.max_bytes = g.saturating_mul(GB);
                true
            })),
            "workspaces.min_free_gb" => Some(number().is_some_and(|g| {
                self.min_free_bytes = g.saturating_mul(GB);
                true
            })),
            _ => None,
        }
    }
}

/// One worktree the node made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub links: MadeLinks,
    /// The checkout it was made from, which `git worktree remove` runs in.
    pub checkout: PathBuf,
    pub branch: String,
    pub created_at_ms: u64,
    /// When the node's pass first saw no agent in it since it was last in use; 0: not seen free yet.
    pub free_since_ms: u64,
    /// Its size when last measured (a lower bound when `size_truncated`).
    pub size_bytes: u64,
    pub size_truncated: bool,
    /// Its build output was removed since it was last in use.
    pub build_output_removed: bool,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

impl Record {
    fn to_json(&self) -> Option<Value> {
        let s = |p: &Path| p.to_str().map(str::to_owned);
        Some(json!({
            "path": s(&self.links.work)?,
            "admin": s(&self.links.admin)?,
            "common": s(&self.links.common)?,
            "dotgit": hex(&self.links.dotgit),
            "commondir": hex(&self.links.commondir),
            "gitdir": hex(&self.links.gitdir),
            "checkout": s(&self.checkout)?,
            "branch": self.branch,
            "created_at_ms": self.created_at_ms,
            "free_since_ms": self.free_since_ms,
            "size_bytes": self.size_bytes,
            "size_truncated": self.size_truncated,
            "build_output_removed": self.build_output_removed,
        }))
    }

    fn from_json(v: &Value) -> Option<Self> {
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        let path = |k: &str| text(k).filter(|t| !t.is_empty()).map(PathBuf::from);
        let bytes = |k: &str| text(k).and_then(unhex);
        let number = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(Self {
            links: MadeLinks {
                work: path("path")?,
                admin: path("admin")?,
                common: path("common")?,
                dotgit: bytes("dotgit")?,
                commondir: bytes("commondir")?,
                gitdir: bytes("gitdir")?,
            },
            checkout: path("checkout")?,
            branch: text("branch")?.to_owned(),
            created_at_ms: number("created_at_ms"),
            free_since_ms: number("free_since_ms"),
            size_bytes: number("size_bytes"),
            size_truncated: flag("size_truncated"),
            build_output_removed: flag("build_output_removed"),
        })
    }
}

/// Who is in which folder: the agents that run or wait to be resumed (busy), and the ones that ended, with when.
#[derive(Debug, Clone, Default)]
pub struct Users {
    busy: Vec<PathBuf>,
    ended: Vec<(PathBuf, u64)>,
    /// Who is where could not be told: every workspace counts as in use.
    unknown: bool,
}

/// `p` in lower case, compared by its parts.
fn folded(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().to_lowercase())
}

/// `folder` and, when it exists, its canonical spelling.
fn spellings(folder: &Path) -> Vec<PathBuf> {
    let mut v = vec![folder.to_path_buf()];
    if let Ok(c) = agora_platform::canonicalize_plain(folder) {
        if c != folder {
            v.push(c);
        }
    }
    v
}

impl Users {
    /// Who is where cannot be told (the node could not be asked): nothing is removed.
    pub fn unknown() -> Self {
        Self {
            unknown: true,
            ..Self::default()
        }
    }

    /// An agent runs, or waits to be resumed or restarted, in `folder`.
    pub fn busy(&mut self, folder: &Path) {
        self.busy.extend(spellings(folder));
    }

    /// An agent that worked in `folder` ended at `at_ms`.
    pub fn ended(&mut self, folder: &Path, at_ms: u64) {
        for f in spellings(folder) {
            self.ended.push((f, at_ms));
        }
    }

    /// From the session ledger: an agent that did not end runs or was interrupted (and may be resumed); one that ended, ended when
    /// its entry was last written.
    pub fn from_ledger(entries: &[crate::ledger::Entry]) -> Self {
        let mut u = Self::default();
        for e in entries.iter().filter(|e| !e.folder.is_empty()) {
            if e.ended {
                u.ended(Path::new(&e.folder), e.updated_at_ms);
            } else {
                u.busy(Path::new(&e.folder));
            }
        }
        u
    }

    /// Whether anyone works at or below `work`, and when the last one there ended. Folders are compared without case on every system:
    /// on one whose disk ignores case a folder may be spelled in another case than the worktree, and on one that does not, a folder that
    /// differs only in case counting as in use only keeps a workspace longer.
    pub fn of(&self, work: &Path) -> Use {
        let work = folded(work);
        let under = |f: &PathBuf| folded(f).starts_with(&work);
        if self.unknown || self.busy.iter().any(under) {
            return Use::Busy;
        }
        Use::Free {
            last_end_ms: self
                .ended
                .iter()
                .filter(|(f, _)| under(f))
                .map(|(_, at)| *at)
                .max(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    Busy,
    Free { last_end_ms: Option<u64> },
}

/// Why a workspace stays ("workspace left behind"). [`Self::code`] is the stable word `agora workspaces` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Uncommitted changes (untracked files included).
    Changes,
    /// A checked-out submodule.
    Submodules,
    /// Commits that exist on no other branch and no remote.
    Unmerged,
    /// Its `.git`, `commondir` or `gitdir` no longer hold what git wrote (T8b): its repository cannot be trusted to answer.
    LinksMoved,
    /// It is no longer a real folder at its recorded place in the node's worktree folder.
    Moved,
    /// git could not answer.
    GitFailed,
    /// git ignores files in it that are not build output or caches ([`needed_ignored`]): they would be lost.
    Ignored,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Changes => "changes",
            Self::Submodules => "submodules",
            Self::Unmerged => "unmerged",
            Self::LinksMoved => "links-moved",
            Self::Moved => "moved",
            Self::GitFailed => "git-failed",
            Self::Ignored => "ignored",
        }
    }

    fn of(k: &Keeps) -> Self {
        match k {
            Keeps::Changes => Self::Changes,
            Keeps::Submodules => Self::Submodules,
            Keeps::Unmerged(_) => Self::Unmerged,
        }
    }

    fn of_error(e: &git::GitError) -> Self {
        match e {
            git::GitError::LinksMoved(_) => Self::LinksMoved,
            git::GitError::Dirty(_) => Self::Changes,
            git::GitError::Unmerged(_) => Self::Unmerged,
            _ => Self::GitFailed,
        }
    }

    /// Git answered and the workspace holds work: its build output may still go (decision 4).
    fn holds_work(self) -> bool {
        matches!(
            self,
            Self::Changes | Self::Submodules | Self::Unmerged | Self::Ignored
        )
    }
}

/// A workspace's state. [`Self::code`] is the stable word `agora workspaces` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// An agent runs or waits to be resumed there.
    InUse,
    /// Its agent ended and it stays: see the reason.
    LeftBehind(Reason),
    /// Nothing would be lost, and the grace period runs until then.
    Waiting { until_ms: u64 },
    /// Nothing would be lost and the grace period passed (or a person asked).
    Cleanable,
}

impl State {
    pub fn code(self) -> &'static str {
        match self {
            Self::InUse => "in-use",
            Self::LeftBehind(_) => "left-behind",
            Self::Waiting { .. } => "waiting",
            Self::Cleanable => "cleanable",
        }
    }
}

/// When the grace period of a free workspace started: the later of when the node first saw it free and the last end of an agent there.
/// Neither known: now (an end that cannot be told is taken as just now).
pub fn grace_from(free_since_ms: u64, last_end_ms: Option<u64>, now_ms: u64) -> u64 {
    match (free_since_ms, last_end_ms) {
        (0, None) => now_ms,
        (seen, end) => seen.max(end.unwrap_or(0)),
    }
}

/// ADR-0064 decision 2, given what is known: in use beats everything; then anything git says would be lost (or could not say) leaves
/// it behind; then the grace period, when it is honoured (the node's own pass; a person who asks does not wait for it).
pub fn decide(
    used: Use,
    work: Result<Option<Keeps>, Reason>,
    grace_from_ms: u64,
    now_ms: u64,
    grace_ms: u64,
    honour_grace: bool,
) -> State {
    if used == Use::Busy {
        return State::InUse;
    }
    match work {
        Err(r) => State::LeftBehind(r),
        Ok(Some(k)) => State::LeftBehind(Reason::of(&k)),
        Ok(None) => {
            let until_ms = grace_from_ms.saturating_add(grace_ms);
            if honour_grace && now_ms < until_ms {
                State::Waiting { until_ms }
            } else {
                State::Cleanable
            }
        }
    }
}

/// One workspace as `agora workspaces` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub path: PathBuf,
    pub branch: String,
    pub state: State,
    /// Since when it is free (the start of its grace period); `None` while in use.
    pub free_since_ms: Option<u64>,
    pub size_bytes: u64,
    pub size_truncated: bool,
    /// Left behind for `ignored`: the ignored paths that would be lost.
    pub needed: Vec<String>,
}

/// What a clean-up did with one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Removed,
    /// `--dry-run`: it would have been removed.
    WouldRemove,
    /// It stays; its build output folders (by name) were removed.
    BuildOutputRemoved(Vec<&'static str>),
    /// Its folder was gone already: only the record was dropped.
    Forgotten,
    /// It stays as it is.
    Kept(State),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub path: PathBuf,
    pub branch: String,
    pub action: Action,
    /// Kept for `ignored`: the ignored paths that would be lost.
    pub needed: Vec<String>,
}

/// How a clean-up runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clean {
    /// The node's own pass waits out the grace period and removes build output of left-behind workspaces; a person does not wait.
    pub by_node: bool,
    /// Only workspaces free for at least this long.
    pub older_than_ms: Option<u64>,
    pub dry_run: bool,
}

/// The node's record of the worktrees it made (see the module documentation).
pub struct Workspaces {
    dir: PathBuf,
    root: PathBuf,
    settings: Mutex<Settings>,
    /// Set when a start was refused for the cap: the node's pass runs at once.
    wake: AtomicBool,
    /// Claims this process found with no stamp on its last pass ([`Self::release_stale_claims`]).
    unstamped: Mutex<std::collections::HashSet<PathBuf>>,
}

impl Workspaces {
    /// The registry in `data_dir` (made owner-only) for worktrees made in `worktree_root`.
    pub fn open(data_dir: &Path, worktree_root: &Path) -> io::Result<Self> {
        let dir = data_dir.join(DIR);
        agora_platform::ensure_private_dir(&dir)?;
        Ok(Self::at(data_dir, worktree_root))
    }

    /// The registry in `data_dir` as it is, creating nothing (`agora workspaces`).
    pub fn at(data_dir: &Path, worktree_root: &Path) -> Self {
        Self {
            dir: data_dir.join(DIR),
            root: std::path::absolute(worktree_root).unwrap_or_else(|_| worktree_root.into()),
            settings: Mutex::new(Settings::default()),
            wake: AtomicBool::new(false),
            unstamped: Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub fn settings(&self) -> Settings {
        *self.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_settings(&self, s: Settings) {
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    /// Asks the node's pass to run now; whether it was asked since the last call.
    pub fn wake(&self) {
        self.wake.store(true, Ordering::Release);
    }

    pub fn take_wake(&self) -> bool {
        self.wake.swap(false, Ordering::AcqRel)
    }

    /// The record's file name: a hash of the worktree's place (its name is the agent's, but a hash never needs checking).
    fn id(work: &Path) -> String {
        let d = ring::digest::digest(&ring::digest::SHA256, work.as_os_str().as_encoded_bytes());
        hex(&d.as_ref()[..16])
    }

    fn file(&self, work: &Path, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", Self::id(work)))
    }

    /// The node's worktree folder, canonical.
    fn root(&self) -> Option<PathBuf> {
        agora_platform::canonicalize_plain(&self.root).ok()
    }

    /// Records the worktree git just made at `worktree` from `checkout` on `branch` (before any agent runs there). Only a worktree
    /// directly in the node's worktree folder, with the links git wrote ([`git::made_links`]) and plain-text paths, is recorded;
    /// whether it was.
    pub fn record_made(&self, worktree: &Path, checkout: &Path, branch: &str, now_ms: u64) -> bool {
        let Some(links) = git::made_links(worktree) else {
            return false;
        };
        if self.root().as_deref() != links.work.parent() || !links.still_hold() {
            return false;
        }
        let checkout = agora_platform::canonicalize_plain(checkout).unwrap_or(checkout.into());
        let r = Record {
            links,
            checkout,
            branch: branch.to_owned(),
            created_at_ms: now_ms,
            free_since_ms: 0,
            size_bytes: 0,
            size_truncated: false,
            build_output_removed: false,
        };
        self.write(&r).is_ok()
    }

    fn write(&self, r: &Record) -> io::Result<()> {
        let v = r
            .to_json()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        let bytes = serde_json::to_vec(&v).map_err(io::Error::other)?;
        agora_platform::write_private(&self.file(&r.links.work, RECORD_EXT), &bytes)
    }

    /// Drops the record of `worktree` (a start that failed gave it back).
    pub fn forget(&self, worktree: &Path) {
        let work = agora_platform::canonicalize_plain(worktree).unwrap_or(worktree.into());
        let _ = std::fs::remove_file(self.file(&work, RECORD_EXT));
    }

    fn read(path: &Path) -> Option<Record> {
        let bytes = agora_platform::read_small_regular_file_no_follow(path, RECORD_MAX).ok()?;
        let v: Value = serde_json::from_slice(&bytes).ok()?;
        let r = Record::from_json(&v)?;
        // A file under another name than its worktree's is not one the node wrote.
        (path.file_stem()? == Self::id(&r.links.work).as_str()).then_some(r)
    }

    /// The files in the registry with `ext`, at most [`MAX_RECORDS`].
    fn files(&self, ext: &str) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut v: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == ext))
            .take(MAX_RECORDS)
            .collect();
        v.sort();
        v
    }

    /// ADR-0136: what the node's worktrees hold together, as last measured.
    pub fn used_bytes(&self) -> u64 {
        self.records()
            .iter()
            .map(|r| r.size_bytes)
            .fold(0, u64::saturating_add)
    }

    /// The folder new worktrees are made under, as configured.
    pub fn worktree_root(&self) -> &Path {
        &self.root
    }

    /// Every workspace recorded (and not claimed by a clean-up right now).
    pub fn records(&self) -> Vec<Record> {
        self.files(RECORD_EXT)
            .iter()
            .filter_map(|p| Self::read(p))
            .collect()
    }

    /// Whether `folder` is in a workspace a clean-up has claimed: a start or resume there is refused.
    pub fn removing(&self, folder: &Path) -> bool {
        let claims = self.files(CLAIM_EXT);
        if claims.is_empty() {
            return false;
        }
        let names = spellings(folder);
        claims.iter().any(|c| match Self::read(c) {
            Some(r) => names.iter().any(|f| f.starts_with(&r.links.work)),
            // A claim that cannot be read could be any workspace's: refuse.
            None => true,
        })
    }

    /// Gives back claims left by a process that ended mid-way: stamped over an hour ago by a process that no longer runs. A claim with no
    /// stamp (its holder ended between the rename and the stamp) is stamped as nobody's only when this process saw it unstamped on an
    /// earlier pass too, so a holder that is between the two steps right now is never overwritten; it then goes an hour later.
    fn release_stale_claims(&self, now_ms: u64) {
        let mut seen = self.unstamped.lock().unwrap_or_else(|e| e.into_inner());
        let before = std::mem::take(&mut *seen);
        for c in self.files(CLAIM_EXT) {
            let Ok(bytes) = agora_platform::read_small_regular_file_no_follow(&c, RECORD_MAX)
            else {
                continue;
            };
            let Ok(mut v) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            match Stamp::of(&v) {
                Some(stamp) => {
                    if now_ms.saturating_sub(stamp.at_ms) >= CLAIM_STALE_MS && !stamp.alive() {
                        give_back(&c, &c.with_extension(RECORD_EXT));
                    }
                }
                None if before.contains(&c) => {
                    Stamp::nobody(now_ms).put(&mut v);
                    if let Ok(bytes) = serde_json::to_vec(&v) {
                        let _ = agora_platform::write_private(&c, &bytes);
                    }
                }
                None => {
                    seen.insert(c);
                }
            }
        }
    }

    /// Whether the node may make one more worktree: `Err((resource, limit))` for `node.resource_limit` when its worktrees hold the cap
    /// (as last measured) or the disk has less than the minimum free. The node's pass is woken on a refusal.
    pub fn room(&self) -> Result<(), (&'static str, u64)> {
        let s = self.settings();
        if s.max_bytes > 0 {
            let used: u64 = self
                .records()
                .iter()
                .map(|r| r.size_bytes)
                .fold(0, u64::saturating_add);
            if used >= s.max_bytes {
                self.wake();
                return Err(("workspaces_gb", s.max_bytes / GB));
            }
        }
        if s.min_free_bytes > 0 {
            // The worktree folder, or the nearest folder above it that exists.
            let free = self
                .root
                .ancestors()
                .find(|p| p.is_dir())
                .and_then(|p| agora_platform::free_space(p).ok());
            if free.is_some_and(|f| f < s.min_free_bytes) {
                self.wake();
                return Err(("free_disk_gb", s.min_free_bytes / GB));
            }
        }
        Ok(())
    }

    /// The worktree is still a real folder directly in the worktree folder at its recorded place, and its links hold what git wrote.
    fn in_place(&self, r: &Record) -> Result<(), Reason> {
        let work = &r.links.work;
        let meta = std::fs::symlink_metadata(work).map_err(|_| Reason::Moved)?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || agora_platform::is_reparse_point(&meta)
        {
            return Err(Reason::Moved);
        }
        if agora_platform::canonicalize_plain(work).ok().as_ref() != Some(work)
            || self.root().as_deref() != work.parent()
        {
            return Err(Reason::Moved);
        }
        if !r.links.still_hold() || !git::adopt_made(&r.links) {
            return Err(Reason::LinksMoved);
        }
        Ok(())
    }

    /// What removing it would lose, asked of git only once it is in place: work git tracks ([`git::keeps_work`]), then ignored files
    /// that are not build output or caches ([`Reason::Ignored`]).
    fn work_of(&self, r: &Record) -> Result<Option<Keeps>, Reason> {
        self.in_place(r)?;
        let keeps =
            git::keeps_work(&r.checkout, &r.links.work).map_err(|e| Reason::of_error(&e))?;
        if keeps.is_none() && !self.needed_of(r)?.is_empty() {
            return Err(Reason::Ignored);
        }
        Ok(keeps)
    }

    /// The ignored paths in `r` that would be lost ([`needed_ignored`]).
    fn needed_of(&self, r: &Record) -> Result<Vec<String>, Reason> {
        let entries = git::ignored_paths(&r.links.work).map_err(|e| Reason::of_error(&e))?;
        Ok(needed_ignored(&r.links.work, &entries))
    }

    /// For a workspace left behind for `ignored`, its needed paths (for the listing); nothing otherwise.
    fn needed_for(&self, r: &Record, state: State) -> Vec<String> {
        match state {
            State::LeftBehind(Reason::Ignored) => self.needed_of(r).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn missing(r: &Record) -> bool {
        std::fs::symlink_metadata(&r.links.work).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
    }

    /// Every workspace with its state, changing nothing on disk. `by_node`: as the node's pass sees it (the grace period counts).
    pub fn list(&self, users: &Users, now_ms: u64, by_node: bool) -> Vec<Listed> {
        let grace = self.settings().grace_ms;
        let mut out = Vec::new();
        for r in self.records() {
            let used = users.of(&r.links.work);
            let (state, free) = match used {
                Use::Busy => (State::InUse, None),
                Use::Free { last_end_ms } => {
                    let from = grace_from(r.free_since_ms, last_end_ms, now_ms);
                    let state = if Self::missing(&r) {
                        State::LeftBehind(Reason::Moved)
                    } else {
                        decide(used, self.work_of(&r), from, now_ms, grace, by_node)
                    };
                    (state, Some(from))
                }
            };
            let (size_bytes, size_truncated) = match crate::files::disk_usage(&r.links.work) {
                Ok(u) => (u.bytes, u.truncated),
                Err(_) => (r.size_bytes, r.size_truncated),
            };
            out.push(Listed {
                path: r.links.work.clone(),
                branch: r.branch.clone(),
                needed: self.needed_for(&r, state),
                state,
                free_since_ms: free,
                size_bytes,
                size_truncated,
            });
        }
        out
    }

    /// A clean-up pass. `users` is asked once at the start and again for each workspace after it is claimed, so an agent started in it
    /// meanwhile is seen. Returns what was done with each workspace. `stop` ends it between two workspaces.
    pub fn clean(
        &self,
        users: &dyn Fn() -> Users,
        now_ms: u64,
        how: Clean,
        stop: &AtomicBool,
    ) -> Vec<Outcome> {
        let settings = self.settings();
        self.release_stale_claims(now_ms);
        let first = users();
        let mut out = Vec::new();
        for mut r in self.records() {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let (path, branch) = (r.links.work.clone(), r.branch.clone());
            let outcome = |action| Outcome {
                path: path.clone(),
                branch: branch.clone(),
                action,
                needed: Vec::new(),
            };
            if Self::missing(&r) {
                if !how.dry_run {
                    self.forget(&r.links.work);
                    self.log(now_ms, "forgotten", &r, how.by_node);
                }
                out.push(outcome(Action::Forgotten));
                continue;
            }
            let used = first.of(&r.links.work);
            let Use::Free { last_end_ms } = used else {
                // In use again: its grace period starts over when it is free again.
                if how.by_node && (r.free_since_ms != 0 || r.build_output_removed) {
                    r.free_since_ms = 0;
                    r.build_output_removed = false;
                    let _ = self.write(&r);
                }
                if how.by_node {
                    self.measure(&r);
                }
                out.push(outcome(Action::Kept(State::InUse)));
                continue;
            };
            if how.by_node && r.free_since_ms == 0 {
                r.free_since_ms = now_ms;
                let _ = self.write(&r);
            }
            let from = grace_from(r.free_since_ms, last_end_ms, now_ms);
            if how
                .older_than_ms
                .is_some_and(|age| now_ms.saturating_sub(from) < age)
            {
                out.push(outcome(Action::Kept(State::Waiting {
                    until_ms: from.saturating_add(how.older_than_ms.unwrap_or(0)),
                })));
                continue;
            }
            let state = decide(
                used,
                self.work_of(&r),
                from,
                now_ms,
                settings.grace_ms,
                how.by_node,
            );
            let action = match state {
                State::Cleanable if how.dry_run => Action::WouldRemove,
                State::Cleanable if how.by_node && !settings.auto_clean => Action::Kept(state),
                State::Cleanable => self.remove_claimed(&r, users, now_ms, how.by_node),
                State::LeftBehind(reason)
                    if how.by_node
                        && !how.dry_run
                        && settings.auto_clean
                        && reason.holds_work()
                        && !r.build_output_removed
                        && now_ms >= from.saturating_add(settings.grace_ms) =>
                {
                    match self.remove_build_output(&mut r, users, now_ms) {
                        Some(names) if !names.is_empty() => Action::BuildOutputRemoved(names),
                        _ => Action::Kept(state),
                    }
                }
                other => Action::Kept(other),
            };
            if how.by_node && !matches!(action, Action::Removed) && !how.dry_run {
                self.measure(&r);
            }
            let needed = match action {
                Action::Kept(state) => self.needed_for(&r, state),
                _ => Vec::new(),
            };
            out.push(Outcome {
                needed,
                ..outcome(action)
            });
        }
        out
    }

    /// Claims `r` (renames its record so no other process acts on it, and starts and resumes there are refused); `None` when another
    /// process holds it or it is gone.
    fn claim(&self, r: &Record) -> Option<Claim> {
        let record = self.file(&r.links.work, RECORD_EXT);
        let claim = self.file(&r.links.work, CLAIM_EXT);
        agora_platform::rename_no_replace(&record, &claim).ok()?;
        let c = Claim {
            record,
            claim,
            done: false,
            stamp: Stamp::mine(),
        };
        // A rename keeps the record's time of last writing, which may be days old: the claim says itself when and by whom it was made.
        let _ = self.write_claimed(&c, r);
        Some(c)
    }

    /// Removes a cleanable workspace under a claim, after asking again who is in it and checking everything again.
    fn remove_claimed(
        &self,
        r: &Record,
        users: &dyn Fn() -> Users,
        now_ms: u64,
        by_node: bool,
    ) -> Action {
        let Some(mut claim) = self.claim(r) else {
            return Action::Kept(State::InUse);
        };
        if users().of(&r.links.work) == Use::Busy {
            return Action::Kept(State::InUse);
        }
        match self.work_of(r) {
            Ok(None) => {}
            Ok(Some(k)) => return Action::Kept(State::LeftBehind(Reason::of(&k))),
            Err(reason) => return Action::Kept(State::LeftBehind(reason)),
        }
        // Held only to check it is at its place right before it goes (on Windows a held folder cannot be removed).
        match agora_platform::HeldFolder::open(&r.links.work) {
            Ok(held) if held.in_place() => drop(held),
            _ => return Action::Kept(State::LeftBehind(Reason::Moved)),
        }
        match git::remove_worktree(&r.checkout, &r.links.work, false) {
            Ok(()) => {
                claim.finish();
                self.log(now_ms, "removed", r, by_node);
                Action::Removed
            }
            Err(e) => Action::Kept(State::LeftBehind(Reason::of_error(&e))),
        }
    }

    /// ADR-0064 decision 4: removes the default build output folders of a left-behind workspace, under a claim, once.
    fn remove_build_output(
        &self,
        r: &mut Record,
        users: &dyn Fn() -> Users,
        now_ms: u64,
    ) -> Option<Vec<&'static str>> {
        let claim = self.claim(r)?;
        if users().of(&r.links.work) == Use::Busy {
            return None;
        }
        self.in_place(r).ok()?;
        // Held, so that right before each removal the worktree is checked to be the folder at its path, not a link swapped in for it.
        let held = agora_platform::HeldFolder::open(&r.links.work).ok()?;
        let mut removed = Vec::new();
        for (marker, name) in BUILD_OUTPUT {
            let work = &r.links.work;
            if !std::fs::symlink_metadata(work.join(marker)).is_ok_and(|m| m.is_file()) {
                continue;
            }
            let dir = work.join(name);
            let real_dir = |d: &Path| {
                std::fs::symlink_metadata(d).is_ok_and(|m| {
                    m.is_dir()
                        && !m.file_type().is_symlink()
                        && !agora_platform::is_reparse_point(&m)
                })
            };
            if !real_dir(&dir) || !git::ignored_untracked(work, name).unwrap_or(false) {
                continue;
            }
            // Asked again right before: still a real folder, and the worktree still at its place. `remove_dir_all` never follows a
            // link inside it.
            if !real_dir(&dir) || !held.in_place() || self.in_place(r).is_err() {
                continue;
            }
            if std::fs::remove_dir_all(&dir).is_ok() {
                removed.push(name);
            }
        }
        r.build_output_removed = true;
        if !removed.is_empty() {
            self.log(
                now_ms,
                &format!("build-output-removed:{}", removed.join(",")),
                r,
                true,
            );
        }
        // Written through the claim's own name, then given back: the record keeps that it was done.
        let _ = self.write_claimed(&claim, r);
        Some(removed)
    }

    fn write_claimed(&self, claim: &Claim, r: &Record) -> io::Result<()> {
        let mut v = r
            .to_json()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        claim.stamp.put(&mut v);
        let bytes = serde_json::to_vec(&v).map_err(io::Error::other)?;
        agora_platform::write_private(&claim.claim, &bytes)
    }

    /// Measures the workspace and keeps its size in its record (for the cap and the listing).
    fn measure(&self, r: &Record) {
        let Ok(u) = crate::files::disk_usage(&r.links.work) else {
            return;
        };
        if (u.bytes, u.truncated) == (r.size_bytes, r.size_truncated) {
            return;
        }
        // Read again: only a record that is still there (not claimed or forgotten meanwhile) is written.
        let file = self.file(&r.links.work, RECORD_EXT);
        if let Some(mut now) = Self::read(&file) {
            now.size_bytes = u.bytes;
            now.size_truncated = u.truncated;
            let _ = self.write(&now);
        }
    }

    /// One line in `workspaces.log`: time, what, by whom, place, branch.
    fn log(&self, now_ms: u64, what: &str, r: &Record, by_node: bool) {
        use std::io::Write;
        let line = format!(
            "{now_ms}\t{what}\t{}\t{}\t{}\n",
            if by_node { "node" } else { "cli" },
            r.links.work.display(),
            r.branch
        );
        if let Ok(mut f) = agora_platform::open_append_private(&self.dir.join(LOG_FILE)) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

/// A workspace claimed by one clean-up ([`Workspaces::claim`]): given back when dropped unless it was removed.
struct Claim {
    record: PathBuf,
    claim: PathBuf,
    done: bool,
    stamp: Stamp,
}

/// When a claim was made and by which process (its pid and start time, as ADR-0060 tells a process from a later one with its pid).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    at_ms: u64,
    /// 0: nobody (a claim found without a stamp).
    pid: u32,
    started: u64,
}

impl Stamp {
    fn mine() -> Self {
        let pid = std::process::id();
        Self {
            at_ms: crate::grants::now_ms(),
            pid,
            started: agora_platform::process_start_time(pid).unwrap_or(0),
        }
    }

    fn nobody(at_ms: u64) -> Self {
        Self {
            at_ms,
            pid: 0,
            started: 0,
        }
    }

    fn put(&self, v: &mut Value) {
        if let Some(o) = v.as_object_mut() {
            o.insert("claimed_at_ms".into(), json!(self.at_ms));
            o.insert("claimer_pid".into(), json!(self.pid));
            o.insert("claimer_started".into(), json!(self.started));
        }
    }

    fn of(v: &Value) -> Option<Self> {
        let n = |k: &str| v.get(k).and_then(Value::as_u64);
        Some(Self {
            at_ms: n("claimed_at_ms")?,
            pid: u32::try_from(n("claimer_pid")?).ok()?,
            started: n("claimer_started")?,
        })
    }

    /// Whether the process that made the claim still runs: the same pid with the same start time, or, where its start time could not be
    /// read when it claimed, any process with that pid (weaker, but never "gone" while it runs).
    fn alive(&self) -> bool {
        if self.pid == 0 {
            return false;
        }
        match agora_platform::process_start_time(self.pid) {
            Some(now) => self.started == 0 || now == self.started,
            None => false,
        }
    }
}

impl Claim {
    /// The workspace is gone: so is its record.
    fn finish(&mut self) {
        self.done = true;
        let _ = std::fs::remove_file(&self.claim);
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if !self.done {
            give_back(&self.claim, &self.record);
        }
    }
}

/// Puts a claimed record back under its own name. When a record was written there meanwhile (the other process's pass wrote what it
/// saw), that one stays and the claim goes: a claim left in place would refuse starts in that workspace for good.
fn give_back(claim: &Path, record: &Path) {
    if let Err(e) = agora_platform::rename_no_replace(claim, record) {
        if e.kind() == io::ErrorKind::AlreadyExists {
            let _ = std::fs::remove_file(claim);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const H: u64 = HOUR_MS;

    #[test]
    fn only_build_output_and_caches_count_as_disposable() {
        let work = std::env::temp_dir().join(format!("agora-ws-disposable-{}", std::process::id()));
        std::fs::create_dir_all(work.join("crate")).unwrap();
        std::fs::write(work.join("crate/Cargo.toml"), "").unwrap();
        let entries: Vec<String> = [
            "node_modules/",
            "web/dist/",
            "build/",
            "out/",
            ".next/",
            ".nuxt/",
            ".turbo/",
            ".cache/",
            ".parcel-cache/",
            "coverage/",
            "pkg/__pycache__/",
            ".pytest_cache/",
            ".mypy_cache/",
            ".ruff_cache/",
            ".gradle/",
            "a/b.pyc",
            "logs/run.log",
            ".DS_Store",
            "crate/target/",
            // Needed:
            ".env",
            ".env.local",
            "data/app.db",
            "x.sqlite3",
            "key.pem",
            "id.key",
            "notes/",
            "target/",
            "my.log.txt",
            ".log",
            "dist",
            "logs/run.log/",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            needed_ignored(&work, &entries),
            vec![
                ".env",
                ".env.local",
                "data/app.db",
                "x.sqlite3",
                "key.pem",
                "id.key",
                "notes/",
                "target/",
                "my.log.txt",
                ".log",
                "dist",
                "logs/run.log/",
            ]
        );
        assert_eq!(DISPOSABLE_DIRS.len(), 15);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn a_folder_in_use_is_found_whatever_its_case() {
        let mut u = Users::default();
        u.busy(Path::new("/wt/Claude-1/src"));
        assert_eq!(u.of(Path::new("/WT/claude-1")), Use::Busy);
        let mut e = Users::default();
        e.ended(Path::new("/WT/CLAUDE-1"), 7);
        assert_eq!(
            e.of(Path::new("/wt/claude-1")),
            Use::Free {
                last_end_ms: Some(7)
            }
        );
    }

    #[test]
    fn in_use_wins_over_everything() {
        for work in [Ok(None), Ok(Some(Keeps::Changes)), Err(Reason::LinksMoved)] {
            assert_eq!(
                decide(Use::Busy, work, 0, 100 * H, 24 * H, true),
                State::InUse
            );
        }
    }

    #[test]
    fn anything_git_says_would_be_lost_leaves_it_behind() {
        let free = Use::Free { last_end_ms: None };
        for (work, reason) in [
            (Ok(Some(Keeps::Changes)), Reason::Changes),
            (Ok(Some(Keeps::Submodules)), Reason::Submodules),
            (
                Ok(Some(Keeps::Unmerged("agent/x".into()))),
                Reason::Unmerged,
            ),
            (Err(Reason::LinksMoved), Reason::LinksMoved),
            (Err(Reason::Moved), Reason::Moved),
            (Err(Reason::GitFailed), Reason::GitFailed),
        ] {
            // Long after the grace period, and by hand: still kept.
            for honour in [true, false] {
                assert_eq!(
                    decide(free, work.clone(), 0, 1000 * H, 24 * H, honour),
                    State::LeftBehind(reason)
                );
            }
        }
    }

    #[test]
    fn a_clean_workspace_waits_out_the_grace_period_only_for_the_node() {
        let free = Use::Free { last_end_ms: None };
        let from = 10 * H;
        assert_eq!(
            decide(free, Ok(None), from, from + 23 * H, 24 * H, true),
            State::Waiting {
                until_ms: from + 24 * H
            }
        );
        assert_eq!(
            decide(free, Ok(None), from, from + 24 * H, 24 * H, true),
            State::Cleanable
        );
        assert_eq!(
            decide(free, Ok(None), from, from + H, 24 * H, false),
            State::Cleanable
        );
    }

    #[test]
    fn the_grace_period_starts_at_the_later_of_first_seen_free_and_the_last_end() {
        // Nothing known: now.
        assert_eq!(grace_from(0, None, 50), 50);
        assert_eq!(grace_from(0, Some(20), 50), 20);
        assert_eq!(grace_from(10, Some(20), 50), 20);
        assert_eq!(grace_from(30, Some(20), 50), 30);
    }

    #[test]
    fn users_are_counted_at_or_below_the_workspace() {
        let mut u = Users::default();
        u.ended(Path::new("/w/a"), 5);
        u.ended(Path::new("/w/a/sub"), 9);
        u.busy(Path::new("/w/b/inner"));
        assert_eq!(
            u.of(Path::new("/w/a")),
            Use::Free {
                last_end_ms: Some(9)
            }
        );
        assert_eq!(u.of(Path::new("/w/b")), Use::Busy);
        // A sibling whose name starts the same is not inside.
        assert_eq!(u.of(Path::new("/w/b2")), Use::Free { last_end_ms: None });
        assert_eq!(Users::unknown().of(Path::new("/w/a")), Use::Busy);
    }

    #[test]
    fn the_ledger_makes_running_and_interrupted_agents_busy() {
        let e = |id: &str, folder: &str, ended: bool| crate::ledger::Entry {
            agent_id: id.into(),
            folder: folder.into(),
            ended,
            updated_at_ms: 7,
            ..Default::default()
        };
        let u = Users::from_ledger(&[e("agent-1", "/w/a", true), e("agent-2", "/w/b", false)]);
        assert_eq!(
            u.of(Path::new("/w/a")),
            Use::Free {
                last_end_ms: Some(7)
            }
        );
        assert_eq!(u.of(Path::new("/w/b")), Use::Busy);
    }

    #[test]
    fn settings_take_their_policy_lines_and_keep_defaults_on_a_bad_value() {
        let mut s = Settings::default();
        assert_eq!(s.take("workspaces.grace_hours", " 2 "), Some(true));
        assert_eq!(s.grace_ms, 2 * H);
        assert_eq!(s.take("workspaces.max_gb", "lots"), Some(false));
        assert_eq!(s.max_bytes, 50 * GB);
        assert_eq!(s.take("workspaces.clean", "off"), Some(true));
        assert!(!s.auto_clean);
        // A word it does not know never leaves automatic clean-up on.
        assert_eq!(s.take("workspaces.clean", "yes"), Some(false));
        assert!(!s.auto_clean);
        assert_eq!(s.take("resume", "auto"), None);
    }

    #[test]
    fn a_record_reads_back_as_written() {
        let r = Record {
            links: MadeLinks {
                work: "/w/x".into(),
                admin: "/r/.git/worktrees/x".into(),
                common: "/r/.git".into(),
                dotgit: b"gitdir: /r/.git/worktrees/x\n".to_vec(),
                commondir: b"../..\n".to_vec(),
                gitdir: b"/w/x/.git\n".to_vec(),
            },
            checkout: "/r".into(),
            branch: "agent/x".into(),
            created_at_ms: 1,
            free_since_ms: 2,
            size_bytes: 3,
            size_truncated: true,
            build_output_removed: true,
        };
        assert_eq!(Record::from_json(&r.to_json().unwrap()), Some(r));
        assert_eq!(unhex("0g"), None);
        assert_eq!(unhex("abc"), None);
    }
}
