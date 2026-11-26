//! Token usage read from the files an agent writes itself (ADR-0042). The manifest says where the file is and how to read its
//! lines (`agora-adapters::usage`); this module finds the files that belong to a running agent, follows their growth and returns
//! what each new line added. A file belongs to an agent when it was touched after the agent started and the working directory it
//! records is the agent's worktree, so another session of the same tool is never counted.
//!
//! Two agents can share a folder (ADR-0052) and the same files (one kind, or kinds with one glob): there the folder says nothing about
//! which session is whose, and both used to count both. So:
//! - a file whose tokens one agent counted is that agent's for good: no other agent counts it, also after its owner ended;
//! - an agent with such a peer counts the tokens of a file only when the file's name holds its own native session id (`<id>.jsonl` for
//!   Claude Code, `rollout-…-<id>.jsonl` for Codex; ADR-0042 point 2). Without its id it reads such files for the tool's plan windows
//!   only (ADR-0091: per tool, not per session) and counts no tokens: a session counted for nobody rather than for two.
//!
//! Known limits: peers are agents with the same glob and the same folder as written (no file system lookup on the supervision
//! thread), so two kinds whose different globs match the same files, or one folder spelled two ways, are not seen as peers; the id is
//! found in the file's name, not read from its lines; an agent that ends in the same second a new peer starts can still take that
//! peer's first file in its last read.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use agora_adapters::manifest::{QuotaSpec, UsageFile};
use agora_adapters::usage::{FileReader, QuotaRaw, Sample, MAX_LINE};
use agora_core::AgentId;

/// Files followed per agent.
const MAX_FILES: usize = 4;
/// Most bytes read for one agent in one step (steps are one second apart).
const READ_BUDGET: u64 = 1024 * 1024;
const READ_EVERY: Duration = Duration::from_secs(1);
const SCAN_EVERY: Duration = Duration::from_secs(5);
/// Usage lines held back while a file's directory is not known yet.
const MAX_PENDING: usize = 1000;
/// Files looked at, and entries read per directory, when expanding a glob.
const MAX_MATCHES: usize = 2000;
const MAX_ENTRIES: usize = 5000;
/// Plan windows held back while a file's directory is not known yet.
const MAX_PENDING_QUOTA: usize = 16;
/// A file may have been created this long before the node noticed the agent.
const SLACK: Duration = Duration::from_secs(2);

/// A running agent whose kind says where its usage file is.
pub struct Target {
    pub agent: AgentId,
    pub worktree: PathBuf,
    pub spec: UsageFile,
    /// ADR-0091: the agent kind the plan windows of this file belong to, and how to read them (`[quota]`).
    pub tool: String,
    pub quota: Option<QuotaSpec>,
    /// The agent's own session id, when the node knows it (chosen at start, or found by `session_files`).
    pub session: Option<String>,
    /// ADR-0021: the kind's `[usage.model_map]`, applied to the model of every sample read for this agent.
    pub model_map: BTreeMap<String, String>,
}

/// ADR-0021: the samples just read for `t` carry the price catalog's id of their model (`[usage.model_map]`), so the hub prices them;
/// a model the map does not list keeps the id the agent wrote.
fn catalog_models(samples: &mut [(AgentId, Sample)], t: &Target) {
    if t.model_map.is_empty() {
        return;
    }
    for (_, s) in samples {
        if let Some(id) = t.model_map.get(&s.model) {
            s.model.clone_from(id);
        }
    }
}

/// What a scan needs to know of every live agent to tell whether another shares this one's folder and files.
#[derive(Clone)]
struct Peer {
    agent: AgentId,
    glob: String,
    worktree: PathBuf,
}

impl Peer {
    fn of(t: &Target) -> Self {
        Self {
            agent: t.agent.clone(),
            glob: t.spec.glob.clone(),
            worktree: t.worktree.clone(),
        }
    }
}

/// Who counted a file, and once that agent has ended, which agents its claim still holds against.
struct Claim {
    owner: AgentId,
    glob: String,
    worktree: PathBuf,
    /// `None` while the owner is live; then the peers it had when it ended.
    guards: Option<HashSet<AgentId>>,
}

impl Claim {
    /// Whether this claim keeps `agent` from counting the file.
    fn bars(&self, agent: &AgentId) -> bool {
        &self.owner != agent && self.guards.as_ref().is_none_or(|g| g.contains(agent))
    }
}

/// A session id short enough to be found by chance inside another file's name is not used to tell files apart.
const MIN_SESSION_ID: usize = 8;

struct FileState {
    reader: FileReader,
    offset: u64,
    /// `None` until a line has said which directory the file belongs to.
    owned: Option<bool>,
    pending: Vec<Sample>,
    /// Plan windows read before the directory was known (at most [`MAX_PENDING_QUOTA`]: the newest).
    pending_quota: Vec<QuotaRaw>,
    /// Whether this file's tokens count for the agent; `false` for a file read only for the tool's plan windows (a peer shares the
    /// folder and this agent's session id is not known).
    tokens: bool,
    /// Set once the file was read while nobody could say whose it is: from then on only a file named after this agent's own session
    /// counts its tokens, also when the peer has gone (its exit does not say which session wrote the file).
    ambiguous: bool,
    /// Whether this agent has counted tokens from this file: a file it counts again later goes on from where it is, never from its
    /// start (that would count the same lines twice).
    counted: bool,
    /// Set once a read has reached the end of the file. Until then the file is being replayed from its start: its plan windows are
    /// old news, so only the newest of each is sent and it is stamped with the line's (or the file's) time, not the read time.
    caught_up: bool,
    /// The end of the file when it has no newline yet.
    partial: Vec<u8>,
}

struct AgentUsage {
    first_seen: SystemTime,
    next_scan: Instant,
    next_read: Instant,
    files: BTreeMap<PathBuf, FileState>,
    ignored: HashSet<PathBuf>,
}

pub struct UsageWatcher {
    home: Option<PathBuf>,
    agents: HashMap<AgentId, AgentUsage>,
    /// ADR-0091: plan windows read from files that are an agent's own, as `(tool, window)` in file order, for [`Self::take_quota`].
    quota_out: Vec<(String, QuotaRaw)>,
    /// The agents of the last [`Self::poll`], so the last read of an ended agent ([`Self::finish`]) still knows its peers.
    peers: Vec<Peer>,
    /// Which agent counted the tokens of a file: no other agent counts that file. After its owner ended, the claim holds against the
    /// agents that were its peers then (they could take the file up), and goes when the last of them has ended: an agent started later
    /// may use the same path for a session of its own.
    owners: HashMap<PathBuf, Claim>,
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

impl Default for UsageWatcher {
    fn default() -> Self {
        Self::new(home_dir())
    }
}

fn same_dir(a: &str, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(Path::new(a)) == canon(b)
}

/// The files a glob matches: `*` stands for any one path component's characters, nothing else is special. When a level has more
/// than [`MAX_MATCHES`], the most recently modified are kept.
pub fn expand(pattern: &Path) -> Vec<PathBuf> {
    let mut level: Vec<PathBuf> = vec![PathBuf::new()];
    for comp in pattern.components() {
        let name = comp.as_os_str().to_string_lossy().into_owned();
        let mut next = Vec::new();
        for base in &level {
            if !name.contains('*') {
                let p = base.join(&name);
                if p.exists() || base.as_os_str().is_empty() {
                    next.push(p);
                }
                continue;
            }
            let Ok(rd) = std::fs::read_dir(if base.as_os_str().is_empty() {
                Path::new(".")
            } else {
                base
            }) else {
                continue;
            };
            for e in rd.flatten().take(MAX_ENTRIES) {
                if wild(&name, &e.file_name().to_string_lossy()) {
                    next.push(e.path());
                }
            }
        }
        if next.len() > MAX_MATCHES {
            // More matches than are followed: keep the most recently written (a running agent's file is among them), not
            // whichever the directory happened to list first (audit F1380).
            let mut timed: Vec<_> = next
                .into_iter()
                .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).ok(), p))
                .collect();
            timed.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
            next = timed
                .into_iter()
                .take(MAX_MATCHES)
                .map(|(_, p)| p)
                .collect();
        }
        level = next;
    }
    level.retain(|p| p.is_file());
    level
}

/// `*` matches any run of characters (including none) within a name.
fn wild(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if parts.len() == 1 {
        return pattern == name;
    }
    if !name.starts_with(first) || !name.ends_with(last) || name.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

impl UsageWatcher {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            agents: HashMap::new(),
            quota_out: Vec::new(),
            peers: Vec::new(),
            owners: HashMap::new(),
        }
    }

    /// The plan windows the last steps read, oldest first (ADR-0091). Only files that are an agent's own count, as for tokens.
    pub fn take_quota(&mut self) -> Vec<(String, QuotaRaw)> {
        std::mem::take(&mut self.quota_out)
    }

    /// One step: follows the targets and returns what new lines added, per agent. Cheap when nothing is due.
    pub fn poll(&mut self, now: Instant, targets: &[Target]) -> Vec<(AgentId, Sample)> {
        let live: HashSet<&AgentId> = targets.iter().map(|t| &t.agent).collect();
        self.agents.retain(|id, _| live.contains(id));
        self.peers = targets.iter().map(Peer::of).collect();
        // A claim whose owner just ended holds against the peers it had then; one with no live guard left is let go.
        for c in self.owners.values_mut() {
            if c.guards.is_none() && !live.contains(&c.owner) {
                c.guards = Some(
                    targets
                        .iter()
                        .filter(|t| t.spec.glob == c.glob && t.worktree == c.worktree)
                        .map(|t| t.agent.clone())
                        .collect(),
                );
            }
        }
        self.owners.retain(|_, c| match &c.guards {
            None => true,
            Some(g) => g.iter().any(|a| live.contains(a)),
        });
        if self.owners.len() > MAX_MATCHES {
            self.owners.retain(|_, c| c.guards.is_none());
        }
        let mut out = Vec::new();
        let Some(home) = self.home.clone() else {
            return out;
        };
        let peers = self.peers.clone();
        for t in targets {
            let from = out.len();
            self.step(now, t, &peers, &home, &mut out);
            catalog_models(&mut out[from..], t);
        }
        out
    }

    /// One last read of agents that just ended, so what they wrote in their final second is not lost. Their state is dropped after.
    pub fn finish(&mut self, now: Instant, targets: &[Target]) -> Vec<(AgentId, Sample)> {
        let mut out = Vec::new();
        let Some(home) = self.home.clone() else {
            return out;
        };
        let peers = self.peers.clone();
        for t in targets {
            if let Some(a) = self.agents.get_mut(&t.agent) {
                a.next_scan = now;
                a.next_read = now;
            }
            let from = out.len();
            self.step(now, t, &peers, &home, &mut out);
            catalog_models(&mut out[from..], t);
            self.agents.remove(&t.agent);
        }
        out
    }

    fn step(
        &mut self,
        now: Instant,
        t: &Target,
        peers: &[Peer],
        home: &Path,
        out: &mut Vec<(AgentId, Sample)>,
    ) {
        let a = self
            .agents
            .entry(t.agent.clone())
            .or_insert_with(|| AgentUsage {
                first_seen: SystemTime::now(),
                next_scan: now,
                next_read: now,
                files: BTreeMap::new(),
                ignored: HashSet::new(),
            });
        let peered = has_peer(t, peers);
        let id = t.session.as_deref().filter(|s| s.len() >= MIN_SESSION_ID);
        let named = |path: &Path, id: &str| {
            path.file_name()
                .is_some_and(|n| n.to_string_lossy().contains(id))
        };
        // Whether a file's tokens may count for this agent: always alone in its folder (the directory decides, as before); with a
        // peer only a file named after its own session.
        let counts = |path: &Path| !peered || id.is_some_and(|id| named(path, id));
        if now >= a.next_scan {
            a.next_scan = now + SCAN_EVERY;
            let glob = t.spec.glob.replacen("{home}", &home.to_string_lossy(), 1);
            for path in expand(Path::new(&glob)) {
                if a.files.len() >= MAX_FILES {
                    break;
                }
                if a.files.contains_key(&path) || a.ignored.contains(&path) {
                    continue;
                }
                if self.owners.get(&path).is_some_and(|c| c.bars(&t.agent)) {
                    continue;
                }
                // With its id known, a peer's file is not even opened.
                if peered && id.is_some() && !counts(&path) {
                    continue;
                }
                let fresh = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|m| m + SLACK >= a.first_seen);
                if fresh {
                    let tokens = counts(&path);
                    a.files.insert(
                        path,
                        FileState {
                            reader: FileReader::new(t.spec.clone()).with_quota(t.quota.clone()),
                            offset: 0,
                            owned: None,
                            tokens,
                            ambiguous: !tokens,
                            counted: false,
                            pending: Vec::new(),
                            pending_quota: Vec::new(),
                            caught_up: false,
                            partial: Vec::new(),
                        },
                    );
                }
            }
        }
        if now < a.next_read {
            return;
        }
        a.next_read = now + READ_EVERY;
        let mut drop_files = Vec::new();
        // A file taken while this agent was alone is looked at again once a peer is there, and a file another agent counted is let go.
        for (path, f) in &mut a.files {
            let own_name = id.is_some_and(|id| named(path, id));
            if self.owners.get(path).is_some_and(|c| c.bars(&t.agent)) {
                drop_files.push(path.clone());
            } else if f.tokens && !counts(path) {
                if id.is_some() {
                    drop_files.push(path.clone());
                } else {
                    f.tokens = false;
                    f.ambiguous = true;
                    f.pending.clear();
                }
            } else if !f.tokens && id.is_some() && !own_name {
                // Its id is known now and this is not its file: the slot goes to its own.
                drop_files.push(path.clone());
            } else if !f.tokens && own_name {
                // Named after its own session: its tokens count. Nobody counted them while it was unclear whose the file is (an agent
                // with its id skips another's file; one without counts none), so a file this agent never counted is read again from
                // its start; one it counted before goes on from where it is.
                if f.counted {
                    f.tokens = true;
                } else {
                    *f = FileState {
                        reader: FileReader::new(t.spec.clone()).with_quota(t.quota.clone()),
                        offset: 0,
                        owned: None,
                        tokens: true,
                        ambiguous: true,
                        counted: false,
                        pending: Vec::new(),
                        pending_quota: Vec::new(),
                        caught_up: false,
                        partial: Vec::new(),
                    };
                }
            } else if !f.tokens && !f.ambiguous && counts(path) {
                f.tokens = true;
            }
        }
        let mut budget = READ_BUDGET;
        for (path, f) in &mut a.files {
            if budget == 0 {
                break;
            }
            if drop_files.contains(path) {
                continue;
            }
            let mut quota = Vec::new();
            let samples = read_new(path, f, &mut budget, t, &mut quota);
            self.quota_out
                .extend(quota.into_iter().map(|q| (t.tool.clone(), q)));
            if f.owned == Some(false) {
                drop_files.push(path.clone());
            }
            if f.tokens {
                f.counted |= !samples.is_empty();
                if f.owned == Some(true) {
                    self.owners.entry(path.clone()).or_insert_with(|| Claim {
                        owner: t.agent.clone(),
                        glob: t.spec.glob.clone(),
                        worktree: t.worktree.clone(),
                        guards: None,
                    });
                }
                out.extend(samples.into_iter().map(|s| (t.agent.clone(), s)));
            }
        }
        for p in drop_files {
            a.files.remove(&p);
            if a.ignored.len() < MAX_MATCHES {
                a.ignored.insert(p);
            }
        }
    }
}

/// Whether another live agent reads the same files in the same folder, so the folder alone cannot say which session is whose.
/// Compared as written: no file system lookup here, on the node's supervision thread (see the module's known limits).
fn has_peer(t: &Target, peers: &[Peer]) -> bool {
    peers
        .iter()
        .any(|p| p.agent != t.agent && p.glob == t.spec.glob && p.worktree == t.worktree)
}

/// Reads what was appended to one file and returns the samples that are the agent's.
fn read_new(
    path: &Path,
    f: &mut FileState,
    budget: &mut u64,
    target: &Target,
    quota_out: &mut Vec<QuotaRaw>,
) -> Vec<Sample> {
    let (worktree, spec) = (&target.worktree, &target.spec);
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map_or(0, |m| m.len());
    if len < f.offset {
        // Truncated or replaced: start over.
        *f = FileState {
            reader: FileReader::new(spec.clone()).with_quota(target.quota.clone()),
            offset: 0,
            owned: None,
            tokens: f.tokens,
            ambiguous: f.ambiguous,
            counted: f.counted,
            pending: Vec::new(),
            pending_quota: Vec::new(),
            caught_up: false,
            partial: Vec::new(),
        };
    }
    let want = (len - f.offset).min(*budget);
    if want == 0 || file.seek(SeekFrom::Start(f.offset)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if file.take(want).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    f.offset += buf.len() as u64;
    let replaying = !f.caught_up;
    f.caught_up = f.offset >= len;
    let first_window = quota_out.len();
    let now_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let file_ms = file_time_ms(path).unwrap_or(now_ms).min(now_ms);
    *budget = budget.saturating_sub(buf.len() as u64);
    let mut data = std::mem::take(&mut f.partial);
    data.extend_from_slice(&buf);
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(nl) = data[start..].iter().position(|b| *b == b'\n') {
        let line = &data[start..start + nl];
        start += nl + 1;
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        let sample = f.reader.line(line);
        // Stamped when read: a replayed line with the time it says (or the file's), a live line with now; never later than now.
        let windows: Vec<QuotaRaw> = f
            .reader
            .take_quota()
            .into_iter()
            .map(|mut w| {
                w.line_ms = Some(if replaying {
                    w.line_ms.unwrap_or(file_ms).min(now_ms)
                } else {
                    now_ms
                });
                w
            })
            .collect();
        if f.owned.is_none() {
            if let Some(cwd) = f.reader.cwd() {
                f.owned = Some(same_dir(cwd, worktree));
                if f.owned == Some(true) {
                    out.append(&mut f.pending);
                    quota_out.append(&mut f.pending_quota);
                }
            }
        }
        match f.owned {
            Some(true) => quota_out.extend(windows),
            None => {
                f.pending_quota.extend(windows);
                let extra = f.pending_quota.len().saturating_sub(MAX_PENDING_QUOTA);
                f.pending_quota.drain(..extra);
            }
            Some(false) => {}
        }
        match (f.owned, sample) {
            (Some(true), Some(s)) => out.push(s),
            (None, Some(s)) if f.pending.len() < MAX_PENDING => f.pending.push(s),
            (Some(false), _) => return Vec::new(),
            _ => {}
        }
    }
    // What follows the last newline waits for the rest of its line (a line longer than the limit is dropped).
    if data.len() - start <= MAX_LINE {
        f.partial = data[start..].to_vec();
    }
    if replaying {
        // Only the last report of each window is news; the ones before it were superseded in the file.
        let mut last: Vec<QuotaRaw> = Vec::new();
        for w in quota_out.drain(first_window..) {
            match last.iter_mut().find(|l| l.minutes == w.minutes) {
                Some(l) => *l = w,
                None => last.push(w),
            }
        }
        quota_out.extend(last);
    }
    out
}

/// When the file was last written, in milliseconds since the Unix epoch.
fn file_time_ms(path: &Path) -> Option<u64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_millis() as u64)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_adapters::manifest::{UsageMode, UsageSelect};
    use std::io::Write;

    fn spec() -> UsageFile {
        UsageFile {
            glob: "{home}/logs/*/*.jsonl".into(),
            format: Some("jsonl".into()),
            select: Some(UsageSelect {
                path: "type".into(),
                equals: "assistant".into(),
            }),
            cwd: "cwd".into(),
            model: "model".into(),
            input: "in".into(),
            output: "out".into(),
            cache_read: None,
            cache_write: None,
            dedupe: Some("id".into()),
            mode: UsageMode::Delta,
        }
    }

    /// Built with serde_json: a Windows path has backslashes that a hand-written JSON string would not escape.
    fn line(cwd: &str, id: &str, i: u64, o: u64) -> String {
        serde_json::json!({"type":"assistant","cwd":cwd,"model":"m","id":id,"in":i,"out":o})
            .to_string()
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-usage-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("logs/p")).unwrap();
        d
    }

    fn target(agent: &str, worktree: &Path) -> Target {
        Target {
            agent: AgentId::new(agent),
            worktree: worktree.to_path_buf(),
            spec: spec(),
            tool: "t".into(),
            quota: None,
            session: None,
            model_map: BTreeMap::new(),
        }
    }

    /// Polls far enough apart in (fake) time that every step is due.
    fn step(w: &mut UsageWatcher, t: &[Target], n: u64) -> Vec<(AgentId, Sample)> {
        w.poll(Instant::now() + Duration::from_secs(10 * n), t)
    }

    #[test]
    fn globs_match_within_one_component() {
        assert!(wild("*.jsonl", "a.jsonl"));
        assert!(wild("rollout-*-x.jsonl", "rollout-2026-x.jsonl"));
        assert!(!wild("*.jsonl", "a.json"));
        assert!(!wild("a*b*c", "ac"));
        assert!(wild("*", "anything"));
        let d = tmp("glob");
        std::fs::write(d.join("logs/p/a.jsonl"), "").unwrap();
        std::fs::write(d.join("logs/p/a.txt"), "").unwrap();
        let found = expand(&d.join("logs/*/*.jsonl"));
        assert_eq!(found, vec![d.join("logs/p/a.jsonl")]);
        let _ = std::fs::remove_dir_all(d);
    }

    /// Audit F1380: past the cap of matches the newest files are kept, so a running agent's file is never cut because the directory
    /// happened to list it last (a user with thousands of old sessions).
    #[test]
    fn past_the_cap_of_matches_the_newest_files_are_kept() {
        let d = tmp("cap");
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_577_836_800);
        let mut i = 0;
        for dir in 0..30 {
            std::fs::create_dir_all(d.join(format!("logs/d{dir:02}"))).unwrap();
            for k in 0..70 {
                let f = std::fs::File::create(d.join(format!("logs/d{dir:02}/old-{k:02}.jsonl")))
                    .unwrap();
                f.set_modified(base + Duration::from_secs(i)).unwrap();
                i += 1;
            }
        }
        // The folder listed last, so without a choice its files are the ones past the cap.
        let last = std::fs::read_dir(d.join("logs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .last()
            .unwrap();
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut w = UsageWatcher::new(Some(d.clone()));
        let t = [target("a1", &work)];
        step(&mut w, &t, 0);
        std::fs::write(
            last.join("own.jsonl"),
            format!("{}\n", line(&work.to_string_lossy(), "m1", 7, 3)),
        )
        .unwrap();
        let found = expand(&d.join("logs/*/*.jsonl"));
        assert_eq!(found.len(), MAX_MATCHES);
        assert!(found.contains(&last.join("own.jsonl")));
        // The oldest are the ones left out.
        assert!(!found.contains(&d.join("logs/d00/old-00.jsonl")));
        let got = step(&mut w, &t, 1);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].1.input, 7);
        let _ = std::fs::remove_dir_all(d);
    }

    /// ADR-0021: a model the kind's `[usage.model_map]` lists is reported by the catalog's id, also in an ended agent's last read; one
    /// it does not list keeps the agent's id.
    #[test]
    fn the_kinds_model_map_names_the_catalog_model() {
        let d = tmp("model-map");
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut w = UsageWatcher::new(Some(d.clone()));
        let mut a = target("a1", &work);
        a.model_map
            .insert("m".to_string(), "example/m-2026".to_string());
        let t = [a];
        assert!(step(&mut w, &t, 0).is_empty());
        let file = d.join("logs/p/s.jsonl");
        let mut f = std::fs::File::create(&file).unwrap();
        writeln!(f, "{}", line(&work.to_string_lossy(), "x1", 4, 1)).unwrap();
        let got = step(&mut w, &t, 1);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].1.model, "example/m-2026");
        let other = serde_json::json!({"type":"assistant","cwd":work.to_string_lossy(),"model":"z","id":"x2","in":1,"out":1});
        writeln!(f, "{other}").unwrap();
        let got = w.finish(Instant::now() + Duration::from_secs(100), &t);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].1.model, "z", "not in the map: as the agent wrote it");
        // Negative control: without the map the same line keeps its own id.
        let mut w = UsageWatcher::new(Some(d.clone()));
        let plain = [target("a2", &work)];
        let _ = std::fs::remove_file(&file);
        assert!(step(&mut w, &plain, 0).is_empty());
        std::fs::write(
            &file,
            format!("{}\n", line(&work.to_string_lossy(), "x3", 4, 1)),
        )
        .unwrap();
        let got = step(&mut w, &plain, 1);
        assert_eq!(got[0].1.model, "m");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn follows_the_agents_own_file_and_reports_only_new_lines_once() {
        let d = tmp("own");
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut w = UsageWatcher::new(Some(d.clone()));
        let t = [target("a1", &work)];
        assert!(step(&mut w, &t, 0).is_empty(), "no file yet");
        let file = d.join("logs/p/s.jsonl");
        let mut f = std::fs::File::create(&file).unwrap();
        writeln!(f, "{}", line(&work.to_string_lossy(), "m1", 10, 2)).unwrap();
        writeln!(f, "{}", line(&work.to_string_lossy(), "m1", 10, 2)).unwrap();
        let got = step(&mut w, &t, 1);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!((got[0].1.input, got[0].1.output), (10, 2));
        assert!(step(&mut w, &t, 2).is_empty(), "nothing new");
        // A line is reported when it is complete, not before.
        let more = line(&work.to_string_lossy(), "m2", 5, 1);
        write!(f, "{}", &more[..20]).unwrap();
        assert!(step(&mut w, &t, 3).is_empty());
        writeln!(f, "{}", &more[20..]).unwrap();
        let got = step(&mut w, &t, 4);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1.input, 5);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn two_agents_in_one_folder_each_count_only_their_own_session() {
        // ADR-0052: agents run in the selected folder, so two agents of one kind can share it; each writes its own session file.
        let d = tmp("shared");
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut w = UsageWatcher::new(Some(d.clone()));
        let one = "11111111-aaaa-4bbb-8ccc-000000000001";
        let two = "22222222-aaaa-4bbb-8ccc-000000000002";
        let with = |agent: &str, id: Option<&str>| Target {
            session: id.map(str::to_string),
            model_map: BTreeMap::new(),
            ..target(agent, &work)
        };
        // The second agent's id is not known yet (Codex: found later, or never when its folder is shared).
        let mut t = [with("a1", Some(one)), with("a2", None)];
        step(&mut w, &t, 0);
        let cwd = work.to_string_lossy();
        std::fs::write(
            d.join(format!("logs/p/{one}.jsonl")),
            format!("{}\n", line(&cwd, "m1", 10, 1)),
        )
        .unwrap();
        std::fs::write(
            d.join(format!("logs/p/rollout-2026-11-21-{two}.jsonl")),
            format!("{}\n", line(&cwd, "m2", 20, 2)),
        )
        .unwrap();
        let got = step(&mut w, &t, 1);
        let total: u64 = got.iter().map(|(_, s)| s.input).sum();
        assert_eq!(
            total, 10,
            "each session counted once at most, and only for its own agent: {got:?}"
        );
        assert!(
            got.iter().all(|(a, s)| a.as_str() == "a1" && s.input == 10),
            "{got:?}"
        );
        // Once its id is known, the second agent counts its own session, and still not the first one's.
        t[1] = with("a2", Some(two));
        let got = step(&mut w, &t, 2);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!((got[0].0.as_str(), got[0].1.input), ("a2", 20));
        // An agent alone in its folder needs no id (unchanged).
        let alone = d.join("alone");
        std::fs::create_dir_all(&alone).unwrap();
        let mut w2 = UsageWatcher::new(Some(d.clone()));
        let t2 = [target("a3", &alone)];
        step(&mut w2, &t2, 0);
        std::fs::write(
            d.join("logs/p/x.jsonl"),
            format!("{}\n", line(&alone.to_string_lossy(), "m3", 5, 1)),
        )
        .unwrap();
        let got = step(&mut w2, &t2, 1);
        assert_eq!(got.iter().map(|(_, s)| s.input).sum::<u64>(), 5, "{got:?}");
        let _ = std::fs::remove_dir_all(d);
    }

    /// Codex review of #881: a survivor never counts a stopped peer's file, a file taken while alone is looked at again when a peer
    /// comes, and a peer without its id still gives the tool's plan windows (no tokens).
    #[test]
    fn a_peers_file_stays_the_peers_and_plan_windows_still_come_without_an_id() {
        let d = tmp("peer-edges");
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_string_lossy().into_owned();
        let one = "11111111-aaaa-4bbb-8ccc-000000000001";
        let two = "22222222-aaaa-4bbb-8ccc-000000000002";
        let with = |agent: &str, id: Option<&str>| Target {
            session: id.map(str::to_string),
            model_map: BTreeMap::new(),
            ..target(agent, &work)
        };
        // a1 alone takes its file; then a2 comes, writes its own, then stops: a1 never counts a2's file.
        let mut w = UsageWatcher::new(Some(d.clone()));
        step(&mut w, &[with("a1", Some(one))], 0);
        std::fs::write(
            d.join(format!("logs/p/{one}.jsonl")),
            format!("{}\n", line(&cwd, "m1", 1, 1)),
        )
        .unwrap();
        assert_eq!(step(&mut w, &[with("a1", Some(one))], 1).len(), 1);
        let both = [with("a1", Some(one)), with("a2", Some(two))];
        step(&mut w, &both, 2);
        std::fs::write(
            d.join(format!("logs/p/{two}.jsonl")),
            format!("{}\n", line(&cwd, "m2", 20, 2)),
        )
        .unwrap();
        let got = step(&mut w, &both, 3);
        assert_eq!(
            got.iter()
                .map(|(a, s)| (a.as_str(), s.input))
                .collect::<Vec<_>>(),
            [("a2", 20)]
        );
        for n in 4..7 {
            let got = step(&mut w, &[with("a1", Some(one))], n);
            assert!(
                got.is_empty(),
                "the survivor counted its stopped peer's file: {got:?}"
            );
        }
        // A file taken while alone and without an id stops counting tokens once a peer is there.
        let mut w = UsageWatcher::new(Some(d.clone()));
        let solo = d.join("logs/p/solo.jsonl");
        step(&mut w, &[with("b1", None)], 0);
        std::fs::write(&solo, format!("{}\n", line(&cwd, "s1", 3, 3))).unwrap();
        assert_eq!(step(&mut w, &[with("b1", None)], 1).len(), 1);
        let pair = [with("b1", None), with("b2", None)];
        step(&mut w, &pair, 2);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&solo)
            .unwrap();
        writeln!(f, "{}", line(&cwd, "s2", 4, 4)).unwrap();
        assert!(
            step(&mut w, &pair, 3).is_empty(),
            "no tokens are counted while nobody knows whose the session is"
        );
        // Plan windows are per tool: a peer without its id still reads them.
        let mut w = UsageWatcher::new(Some(d.clone()));
        let pair = [
            Target {
                session: None,
                model_map: BTreeMap::new(),
                ..quota_target("c1", &work)
            },
            Target {
                session: None,
                model_map: BTreeMap::new(),
                ..quota_target("c2", &work)
            },
        ];
        step(&mut w, &pair, 0);
        std::fs::write(
            d.join("logs/p/q.jsonl"),
            format!("{}\n", qline(Some(&cwd), 42)),
        )
        .unwrap();
        assert!(step(&mut w, &pair, 1).is_empty(), "no tokens");
        let seen: Vec<_> = w
            .take_quota()
            .iter()
            .map(|(t, q)| (t.clone(), q.percent))
            .collect();
        assert!(seen.contains(&("tool-x".to_string(), 42.0)), "{seen:?}");
        let _ = std::fs::remove_dir_all(d);
    }

    /// Codex review 2 of #881: peers coming and going never make a file count twice, a known id frees the slots of others' files, and
    /// a path an ended agent used is free for an agent started after it.
    #[test]
    fn peers_coming_and_going_never_count_a_line_twice() {
        let d = tmp("peer-transitions");
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_string_lossy().into_owned();
        let with = |agent: &str, id: Option<&str>| Target {
            session: id.map(str::to_string),
            model_map: BTreeMap::new(),
            ..target(agent, &work)
        };
        let total = |got: &[(AgentId, Sample)], agent: &str| -> u64 {
            got.iter()
                .filter(|(a, _)| a.as_str() == agent)
                .map(|(_, s)| s.input)
                .sum()
        };
        // b1 (no id) counts its file alone; a peer comes and goes; b1 never counts those lines again.
        let mut w = UsageWatcher::new(Some(d.clone()));
        let solo = d.join("logs/p/solo.jsonl");
        step(&mut w, &[with("b1", None)], 0);
        std::fs::write(&solo, format!("{}\n", line(&cwd, "s1", 3, 3))).unwrap();
        let mut counted = total(&step(&mut w, &[with("b1", None)], 1), "b1");
        counted += total(
            &step(&mut w, &[with("b1", None), with("b2", None)], 2),
            "b1",
        );
        for n in 3..6 {
            counted += total(&step(&mut w, &[with("b1", None)], n), "b1");
        }
        assert_eq!(counted, 3, "a line counted twice");
        // Two agents without ids: when one ends, the survivor still counts neither file (the exit says nothing about whose it is).
        let mut w = UsageWatcher::new(Some(d.clone()));
        let pair = [with("c1", None), with("c2", None)];
        step(&mut w, &pair, 0);
        std::fs::write(
            d.join("logs/p/c-x.jsonl"),
            format!("{}\n", line(&cwd, "x1", 5, 5)),
        )
        .unwrap();
        std::fs::write(
            d.join("logs/p/c-y.jsonl"),
            format!("{}\n", line(&cwd, "y1", 6, 6)),
        )
        .unwrap();
        assert!(step(&mut w, &pair, 1).is_empty());
        w.finish(
            Instant::now() + Duration::from_secs(25),
            &[with("c2", None)],
        );
        for n in 3..6 {
            assert!(
                step(&mut w, &[with("c1", None)], n).is_empty(),
                "the survivor took an unclear file"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
        // An agent whose slots are full of others' files reaches its own once its id is known.
        let d = tmp("peer-slots");
        let work2 = d.join("work");
        std::fs::create_dir_all(&work2).unwrap();
        let cwd2 = work2.to_string_lossy().into_owned();
        let with2 = |agent: &str, id: Option<&str>| Target {
            session: id.map(str::to_string),
            model_map: BTreeMap::new(),
            ..target(agent, &work2)
        };
        let own = "33333333-aaaa-4bbb-8ccc-000000000003";
        let mut w = UsageWatcher::new(Some(d.clone()));
        step(&mut w, &[with2("e1", None), with2("e2", None)], 0);
        for i in 0..MAX_FILES {
            std::fs::write(
                d.join(format!("logs/p/other-{i}.jsonl")),
                format!("{}\n", line(&cwd2, &format!("o{i}"), 1, 1)),
            )
            .unwrap();
        }
        step(&mut w, &[with2("e1", None), with2("e2", None)], 1);
        std::fs::write(
            d.join(format!("logs/p/{own}.jsonl")),
            format!("{}\n", line(&cwd2, "own", 9, 9)),
        )
        .unwrap();
        let mut got = 0;
        for n in 2..5 {
            got += total(
                &step(&mut w, &[with2("e1", Some(own)), with2("e2", None)], n),
                "e1",
            );
        }
        assert_eq!(got, 9, "the agent never reached its own file");
        let _ = std::fs::remove_dir_all(&d);
        // A path an ended agent counted is free for an agent started after it (a tool that reuses one log path per session).
        let d = tmp("path-reuse");
        let work3 = d.join("work");
        std::fs::create_dir_all(&work3).unwrap();
        let cwd3 = work3.to_string_lossy().into_owned();
        let fixed = d.join("logs/p/fixed.jsonl");
        let mut w = UsageWatcher::new(Some(d.clone()));
        step(&mut w, &[target("f1", &work3)], 0);
        std::fs::write(&fixed, format!("{}\n", line(&cwd3, "f1", 2, 2))).unwrap();
        assert_eq!(total(&step(&mut w, &[target("f1", &work3)], 1), "f1"), 2);
        w.finish(
            Instant::now() + Duration::from_secs(15),
            &[target("f1", &work3)],
        );
        w.poll(Instant::now() + Duration::from_secs(20), &[]);
        // The tool starts the next session in the same file before the node sees the new agent.
        std::fs::write(&fixed, format!("{}\n", line(&cwd3, "f2", 70, 70))).unwrap();
        let mut got = 0;
        for n in 3..6 {
            got += total(&step(&mut w, &[target("f2", &work3)], n), "f2");
        }
        assert_eq!(got, 70, "a later agent lost the reused path");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn another_sessions_file_in_another_directory_is_never_counted() {
        let d = tmp("foreign");
        let (mine, other) = (d.join("mine"), d.join("other"));
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let mut w = UsageWatcher::new(Some(d.clone()));
        let t = [target("a1", &mine)];
        step(&mut w, &t, 0);
        std::fs::write(
            d.join("logs/p/other.jsonl"),
            format!("{}\n", line(&other.to_string_lossy(), "x1", 99, 99)),
        )
        .unwrap();
        std::fs::write(
            d.join("logs/p/mine.jsonl"),
            format!("{}\n", line(&mine.to_string_lossy(), "y1", 1, 1)),
        )
        .unwrap();
        let got = step(&mut w, &t, 1);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].1.input, 1);
        assert!(
            step(&mut w, &t, 2).is_empty(),
            "the foreign file stays ignored"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_file_older_than_the_agent_is_not_its_own_and_a_gone_agent_is_forgotten() {
        let d = tmp("old");
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let old = d.join("logs/p/old.jsonl");
        std::fs::write(
            &old,
            format!("{}\n", line(&work.to_string_lossy(), "o1", 7, 7)),
        )
        .unwrap();
        let past = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();
        let mut w = UsageWatcher::new(Some(d.clone()));
        let t = [target("a1", &work)];
        assert!(step(&mut w, &t, 0).is_empty());
        assert!(
            step(&mut w, &t, 1).is_empty(),
            "stale file: a different, earlier session"
        );
        w.poll(Instant::now(), &[]);
        assert!(w.agents.is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn lines_before_the_directory_is_known_wait_and_are_counted_once_it_matches() {
        let d = tmp("pending");
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut w = UsageWatcher::new(Some(d.clone()));
        let t = [target("a1", &work)];
        step(&mut w, &t, 0);
        // The first usage line has no `cwd` of its own; a later line says it.
        let first = r#"{"type":"assistant","model":"m","id":"p1","in":3,"out":1}"#;
        let later = serde_json::json!({"type":"user","cwd":work.to_string_lossy()}).to_string();
        std::fs::write(d.join("logs/p/s.jsonl"), format!("{first}\n{later}\n")).unwrap();
        let got = step(&mut w, &t, 1);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].1.input, 3);
        let _ = std::fs::remove_dir_all(d);
    }

    fn quota_target(agent: &str, worktree: &Path) -> Target {
        let mut t = target(agent, worktree);
        t.tool = "tool-x".into();
        t.quota = Some(QuotaSpec {
            at: Some("ts".into()),
            window: vec![agora_adapters::manifest::QuotaWindow {
                percent: "q.p".into(),
                minutes: "q.m".into(),
                resets: "q.r".into(),
            }],
        });
        t
    }

    fn qline(cwd: Option<&str>, p: u64) -> String {
        let mut v = serde_json::json!({"type":"assistant","q":{"p":p,"m":300,"r":1795000000u64}});
        if let Some(c) = cwd {
            v["cwd"] = c.into();
        }
        v.to_string()
    }

    /// ADR-0091: plan windows come only from the agent's own file, also when the line that says the directory comes later, and a file
    /// of another directory never gives one.
    #[test]
    fn plan_windows_are_read_from_the_agents_own_file_only() {
        let d = tmp("quota");
        let (mine, other) = (d.join("mine"), d.join("other"));
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let mut w = UsageWatcher::new(Some(d.clone()));
        let t = [quota_target("a1", &mine)];
        step(&mut w, &t, 0);
        // The first window is read before any line says the directory; the later line does.
        let own = format!(
            "{}\n{}\n{}\n",
            qline(None, 10),
            qline(Some(&mine.to_string_lossy()), 11),
            qline(Some(&mine.to_string_lossy()), 12)
        );
        std::fs::write(d.join("logs/p/mine.jsonl"), own).unwrap();
        std::fs::write(
            d.join("logs/p/other.jsonl"),
            format!("{}\n", qline(Some(&other.to_string_lossy()), 99)),
        )
        .unwrap();
        step(&mut w, &t, 1);
        let got = w.take_quota();
        let seen: Vec<_> = got.iter().map(|(t, q)| (t.as_str(), q.percent)).collect();
        // The file is read from its start: only the newest report of the window is news.
        assert_eq!(seen, [("tool-x", 12.0)]);
        assert!(w.take_quota().is_empty());
        // Lines appended after that are live: each is reported, stamped with the read time.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(d.join("logs/p/mine.jsonl"))
            .unwrap();
        writeln!(f, "{}", qline(Some(&mine.to_string_lossy()), 13)).unwrap();
        writeln!(f, "{}", qline(Some(&mine.to_string_lossy()), 14)).unwrap();
        step(&mut w, &t, 2);
        let seen: Vec<_> = w.take_quota().iter().map(|(_, q)| q.percent).collect();
        assert_eq!(seen, [13.0, 14.0]);
        // A manifest without `[quota]` reads none.
        let mut w = UsageWatcher::new(Some(d.clone()));
        let t = [target("a1", &mine)];
        step(&mut w, &t, 0);
        step(&mut w, &t, 1);
        assert!(w.take_quota().is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    /// ADR-0091 decision 4: a resumed session's file is read from its start, and what it said hours ago is not "observed now".
    #[test]
    fn an_old_file_replays_one_report_per_window_with_its_old_time() {
        let d = tmp("replay");
        let work = d.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_string_lossy().into_owned();
        let now_ms = || {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
        };
        let mut w = UsageWatcher::new(Some(d.clone()));
        let t = [quota_target("a1", &work)];
        step(&mut w, &t, 0);
        let hours = |h: u64| h * 3600 * 1000;
        let rfc = |ms: u64| {
            // Whole seconds; the days come from the standard formula the reader inverts.
            let secs = ms / 1000;
            let (days, rem) = (secs / 86_400, secs % 86_400);
            let (y, m, d) = civil(days as i64);
            format!(
                "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
                rem / 3600,
                rem % 3600 / 60,
                rem % 60
            )
        };
        let (old, older) = (now_ms() - hours(20), now_ms() - hours(21));
        let line = |ts: Option<&str>, p: u64| {
            let mut v = serde_json::json!({"type":"assistant","cwd":cwd,"q":{"p":p,"m":300,"r":(now_ms()/1000)+3600}});
            if let Some(ts) = ts {
                v["ts"] = ts.into();
            }
            v.to_string()
        };
        // A file whose lines carry their time: the last line's time is the stamp.
        let file = d.join("logs/p/old.jsonl");
        std::fs::write(
            &file,
            format!(
                "{}\n{}\n",
                line(Some(&rfc(older)), 20),
                line(Some(&rfc(old)), 21)
            ),
        )
        .unwrap();
        step(&mut w, &t, 1);
        let got = w.take_quota();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].1.percent, 21.0);
        let stamp = got[0].1.line_ms.unwrap();
        assert!(
            stamp.abs_diff(old) < 1000,
            "the line's own time, not now: {stamp} vs {old}"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    /// Civil date from days since 1970-01-01 (the inverse of the reader's formula), for building test timestamps.
    fn civil(z: i64) -> (i64, i64, i64) {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + era * 400 + i64::from(m <= 2), m, d)
    }
}
