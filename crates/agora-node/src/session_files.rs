//! A terminal agent's own session id, read from the session file the agent writes (ADR-0060, manifest `[session.file]`). Used for
//! agents whose id the node cannot choose (Codex: no flag sets it), so that Resume after a node restart can continue the conversation.
//!
//! A file is the agent's when it was written after the agent started, its first line passes `select`, names the agent's folder as its
//! working directory, and its id is no other agent's. This is a guess that can be wrong, so it is made only when it cannot be confused.
//! Two agents waiting in one folder are rivals when they are of one kind, or of kinds whose globs could match the same files: the fixed
//! directory of one glob, before its first `*`, is the other's or holds it, as written or with links resolved (even links whose target
//! does not exist yet), compared without case (one folder on Windows and macOS; elsewhere this only adds rivals). A root that cannot be
//! resolved counts as holding every other. They are not rivals when both `select`s ask for different literal values at the same path.
//! This is decided from the manifests, before any file is read, for every live agent at once whichever is due: rivals get no id at all,
//! so kinds that share a session folder lose Resume in that folder rather than risk continuing the wrong conversation. Neither does an
//! agent for which more than one new file fits (the window then offers Start again). An agent that has no file after
//! [`GIVE_UP_AFTER`] gets none.
//!
//! Known limit: discovery assumes that no link inside a *wildcard* directory of a session glob points into another kind's session root
//! (for example `logs/*/*.jsonl` with `logs/linked` pointing at `store`, against `store/*.jsonl`); finding one would mean reading those
//! directories on every poll. Such a setup can attribute a session to the wrong agent.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use agora_adapters::manifest::SessionFile;
use agora_core::AgentId;
use serde_json::Value;

/// How often the files are looked at while an agent waits for its id.
const SCAN_EVERY: Duration = Duration::from_secs(3);
/// How long an agent waits for its session file (the tool may write it only once it has been asked something).
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(30 * 60);
/// A file may have been written this long before the node noticed the agent.
const SLACK: Duration = Duration::from_secs(2);
/// Longest first line read.
const FIRST_LINE_MAX: u64 = 64 * 1024;
/// Longest id accepted; it is passed to the agent's `[resume]` argv, so only plain characters.
const MAX_ID: usize = 128;

/// A running terminal agent without a session id whose kind names its session file.
#[derive(Debug, Clone)]
pub struct Target {
    pub agent: AgentId,
    pub kind: String,
    pub folder: PathBuf,
    pub spec: SessionFile,
}

struct Waiting {
    since: SystemTime,
    give_up_at: Instant,
    next_scan: Instant,
}

/// A looked-up path is trusted this long; after it the path is resolved again (a workspace moved and its old path made a link would
/// otherwise stay "another folder" for as long as the agent runs, F1180 follow-up). Bounded: one lookup per agent and path per period.
const REVERIFY_AFTER: Duration = Duration::from_secs(30);

/// Which lookup of an agent: its workspace folder, or its session root.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Folder,
    Root,
}

/// One lookup of one path. `value` is `None` while the answer is outstanding (a background probe): the caller then fails closed.
struct Slot {
    /// What was asked: an answer is only good for the same path (a manifest reloaded under a running agent changes the root).
    key: PathBuf,
    at: Instant,
    seq: u64,
    value: Option<Option<PathBuf>>,
}

/// What was looked up on disk for one agent; kept until the agent leaves.
#[derive(Default)]
struct Looked {
    folder: Option<Slot>,
    root: Option<Slot>,
}

/// Asks for a lookup: its sequence number, and the answer when it is there at once (inline probes).
type Asker<'a> = dyn FnMut(&AgentId, Which, &Path) -> (u64, Option<Option<PathBuf>>) + 'a;

struct Ask {
    agent: AgentId,
    which: Which,
    seq: u64,
    path: PathBuf,
}

struct Answer {
    agent: AgentId,
    which: Which,
    seq: u64,
    value: Option<PathBuf>,
}

/// The one thread that probes the disk for a watcher in the node: a probe on a stalled mount blocks only this thread, never the
/// supervision thread, and its answer arrives as a message.
struct Worker {
    tx: std::sync::mpsc::Sender<Ask>,
    rx: std::sync::mpsc::Receiver<Answer>,
}

pub struct SessionWatcher {
    home: Option<PathBuf>,
    waiting: HashMap<AgentId, Waiting>,
    /// Agents that will get no id from a file (ambiguous, or none came in time), until they end.
    gave_up: HashSet<AgentId>,
    /// How a workspace folder and a session root are looked at on disk ([`workspace`], [`resolve`]); replaced in tests to count them.
    probe: fn(&Path) -> PathBuf,
    resolver: fn(&Path) -> Option<PathBuf>,
    /// Each live agent's workspace and session root as looked up on disk, so no agent is probed again in a later pass.
    looked: HashMap<AgentId, Looked>,
    /// Probes run here when set (the node); without it they run inline in `poll` (tests).
    worker: Option<Worker>,
    seq: u64,
}

impl Default for SessionWatcher {
    fn default() -> Self {
        Self::new(crate::usage_files::home_dir()).background()
    }
}

fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, k| v.get(k))
}

/// Whether a session id may be put on an agent's command line (`[resume]` argv, `[session] id_args`): it starts with an ASCII letter or
/// digit (so it can never be read as an option such as `--dangerously-bypass-approvals-and-sandbox`), then only letters, digits, `.`,
/// `_` and `-`, at most 128 characters. The one check every id passes before it reaches an argv (ADR-0060). An adapter's id travels in
/// JSON (`session/load`) and need not pass it, but it never goes on a command line unless it does.
pub fn argv_safe(id: &str) -> bool {
    id.len() <= MAX_ID
        && id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A UUID in its usual spelling (8-4-4-4-12 hex digits): what Codex and Claude Code session ids are. A session file's id must be one.
pub fn is_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// `file`'s first line as JSON, when it is complete within [`FIRST_LINE_MAX`].
fn first_line(file: &Path) -> Option<Value> {
    let f = std::fs::File::open(file).ok()?;
    let mut line = String::new();
    // Two bytes of lookahead tell a complete line (`\n` or `\r\n`) from a valid JSON prefix at the cap.
    BufReader::new(f.take(FIRST_LINE_MAX + 2))
        .read_line(&mut line)
        .ok()?;
    // The terminator ends the line and is not part of it (F1181, Codex review of #791): a line of exactly the cap is accepted whether
    // it ends in LF, in CRLF, or is not ended yet.
    let body = line
        .strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .unwrap_or(&line);
    if body.len() as u64 > FIRST_LINE_MAX {
        return None;
    }
    serde_json::from_str(body.trim_end()).ok()
}

/// Whether a first line passes `spec`'s `select`.
fn selected(v: &Value, spec: &SessionFile) -> bool {
    spec.select
        .as_ref()
        .is_none_or(|sel| at(v, &sel.path).and_then(Value::as_str) == Some(sel.equals.as_str()))
}

/// The id in a first line when that line is a session's start in `folder`.
fn session_of(v: &Value, spec: &SessionFile, folder: &Path) -> Option<String> {
    if !selected(v, spec) {
        return None;
    }
    let cwd = at(v, &spec.cwd)?.as_str()?;
    if !same_dir(Path::new(cwd), folder) {
        return None;
    }
    let id = at(v, &spec.id)?.as_str()?;
    (argv_safe(id) && is_uuid(id)).then(|| id.to_string())
}

/// The id in `file`'s first line when that line is a session's start in `folder`.
fn session_in(file: &Path, spec: &SessionFile, folder: &Path) -> Option<String> {
    session_of(&first_line(file)?, spec, folder)
}

/// The fixed directory part of a session glob: its directories before the first one with a `*` (the file name never counts), with
/// `{home}` replaced. Returned twice: with `.` and `..` resolved as written (no I/O), for the as-written comparison, and untouched, for
/// [`resolve`], which follows a link before stepping to `..` the way the filesystem (and `expand`) does (Codex review of #791).
fn session_root(glob: &str, home: &Path) -> (PathBuf, PathBuf) {
    let glob = glob.replacen("{home}", &home.to_string_lossy(), 1);
    let glob = Path::new(&glob);
    let (mut lexical, mut raw) = (PathBuf::new(), PathBuf::new());
    for comp in glob.parent().map(Path::components).into_iter().flatten() {
        if comp.as_os_str().to_string_lossy().contains('*') {
            break;
        }
        raw.push(comp);
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                lexical.pop();
            }
            c => lexical.push(c),
        }
    }
    (lexical, raw)
}

/// Most links followed while resolving one root (as Linux's `MAXSYMLINKS`).
const MAX_LINK_HOPS: usize = 40;

/// `path` with its links resolved. When it exists, the OS resolves it (`canonicalize`, with each platform's own rules, such as Windows
/// taking `..` before a junction). When part of it does not exist yet, it is walked one component at a time instead: a link is read and
/// followed even when what it points to does not exist yet (a relative target from the link's folder), and the folders that do not
/// exist yet are kept as written, so a root resolves through a link before its own folder is made (Codex review of #791). Where the walk
/// and the OS could differ (`..` after a link on Windows), the OS's answer is the lexical one, which is compared too. `None` when it
/// cannot be known (more than [`MAX_LINK_HOPS`] links, or an error other than "not found"): the caller then fails closed.
fn resolve(path: &Path) -> Option<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(c) => Some(c),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => walk(path),
        Err(_) => None,
    }
}

fn walk(path: &Path) -> Option<PathBuf> {
    // One component per entry, so a link's target can be put in front of what is left.
    let mut todo: VecDeque<PathBuf> = path
        .components()
        .map(|c| PathBuf::from(c.as_os_str()))
        .collect();
    let mut out = PathBuf::new();
    let mut hops = 0;
    let mut exists = true;
    while let Some(part) = todo.pop_front() {
        match part.components().next() {
            None | Some(Component::CurDir) => {}
            // An absolute target starts again from its root (`push` replaces).
            Some(Component::Prefix(_)) | Some(Component::RootDir) => out.push(&part),
            Some(Component::ParentDir) => {
                out.pop();
            }
            Some(Component::Normal(name)) => {
                let candidate = out.join(name);
                if !exists {
                    out = candidate;
                    continue;
                }
                match std::fs::symlink_metadata(&candidate) {
                    Ok(m) if m.file_type().is_symlink() => {
                        hops += 1;
                        if hops > MAX_LINK_HOPS {
                            return None;
                        }
                        let target = std::fs::read_link(&candidate).ok()?;
                        for c in target.components().rev() {
                            todo.push_front(PathBuf::from(c.as_os_str()));
                        }
                    }
                    Ok(_) => out = candidate,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        exists = false;
                        out = candidate;
                    }
                    Err(_) => return None,
                }
            }
        }
    }
    // The longest existing part in the filesystem's own spelling (case on Windows and macOS), the folders not made yet as written.
    for ancestor in out.ancestors() {
        if let Ok(c) = std::fs::canonicalize(ancestor) {
            return Some(match out.strip_prefix(ancestor) {
                Ok(rest) if !rest.as_os_str().is_empty() => c.join(rest),
                _ => c,
            });
        }
    }
    Some(out)
}

/// Whether two agents' `select`s ask for different literal values at the same path: no first line can pass both.
fn exclusive(a: &SessionFile, b: &SessionFile) -> bool {
    matches!((&a.select, &b.select), (Some(x), Some(y)) if x.path == y.path && x.equals != y.equals)
}

/// Whether one path is the other or holds it, compared without case: on a case-insensitive filesystem (NTFS, APFS by default) two
/// spellings are one folder, and the parts not made yet cannot be asked. Doing it everywhere needs no platform code and only adds
/// rivals on a case-sensitive one (fails closed).
fn nested(x: &Path, y: &Path) -> bool {
    let fold = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    let (x, y) = (fold(x), fold(y));
    x.starts_with(&y) || y.starts_with(&x)
}

/// Every live target that has a rival in its folder. Two agents in one folder are rivals when they are of one kind, or when their
/// globs' fixed directories are the same or one holds the other (as written or with links resolved: either is enough, since `expand`
/// follows links below the fixed part; a root that cannot be resolved counts as holding every other), unless their `select`s are
/// exclusive. Computed over all of them at once, whatever is due, so no pair is missed because one of its agents was already set aside
/// (Codex review of #791). Not closed transitively: an agent without a rival of its own shares no possible file with any live agent,
/// and every rival of an agent in the set is in the set itself. Roots are resolved only for pairs that need it (two kinds in one
/// folder, not exclusive, disjoint as written), so a root on a stalled mount never holds up an agent with no such neighbour. The probes
/// run on a worker thread in the node (`SessionWatcher::background`), never on the supervision thread; until an answer is there the
/// pair is left undecided and its agents get no session.
#[cfg(test)]
fn with_rivals(targets: &[Target], home: &Path) -> HashSet<AgentId> {
    with_rivals_using(
        targets,
        home,
        &HashSet::new(),
        &mut HashMap::new(),
        &mut workspace,
        &mut |p| resolve(p),
    )
}

/// [`compare_rivals`] with the probes answered inline by the given functions.
#[cfg(test)]
fn with_rivals_using(
    targets: &[Target],
    home: &Path,
    settled: &HashSet<AgentId>,
    looked: &mut HashMap<AgentId, Looked>,
    probe: &mut dyn FnMut(&Path) -> PathBuf,
    resolver: &mut dyn FnMut(&Path) -> Option<PathBuf>,
) -> HashSet<AgentId> {
    compare_rivals(
        targets,
        home,
        settled,
        looked,
        Instant::now(),
        &mut |_, which, p| {
            let v = match which {
                Which::Folder => Some(probe(p)),
                Which::Root => resolver(p),
            };
            (0, Some(v))
        },
    )
    .0
}

/// A workspace folder as the filesystem names it (the comparison `same_dir` makes); probed only for agents that could contend.
fn workspace(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The answer for `key` in `slot`, asking again (through `ask`) when there is none for this path or it is older than
/// [`REVERIFY_AFTER`]. `None` while the answer is outstanding.
fn lookup(
    slot: &mut Option<Slot>,
    key: &Path,
    now: Instant,
    agent: &AgentId,
    which: Which,
    ask: &mut Asker<'_>,
) -> Option<Option<PathBuf>> {
    let fresh = matches!(slot, Some(s) if s.key == key
        && (s.value.is_none() || now.saturating_duration_since(s.at) < REVERIFY_AFTER));
    if !fresh {
        let (seq, value) = ask(agent, which, key);
        *slot = Some(Slot {
            key: key.to_path_buf(),
            at: now,
            seq,
            value,
        });
    }
    slot.as_ref().and_then(|s| s.value.clone())
}

/// The rivals, and the agents whose comparison waits for an answer of the background probe (they get no session until it comes, and
/// are not given up on).
fn compare_rivals(
    targets: &[Target],
    home: &Path,
    settled: &HashSet<AgentId>,
    looked: &mut HashMap<AgentId, Looked>,
    now: Instant,
    ask: &mut Asker<'_>,
) -> (HashSet<AgentId>, HashSet<AgentId>) {
    // A workspace is probed only for a pair that could contend (one kind, or two kinds whose `select`s are not exclusive), the pairs
    // `same_dir` always compared: a lone agent, or one whose only neighbours have exclusive selectors, is never probed, so a stalled
    // mount under it never holds up discovery. Folders that are equal as written need no probe. A pair whose agents had both given up
    // (`settled`) is not compared again: neither gets an id anyway; each is still compared with the agents that wait.
    // Each agent's lookups are kept in `looked` and trusted for [`REVERIFY_AFTER`] and for the same path only.
    let mut out = HashSet::new();
    let mut pending = HashSet::new();
    for i in 0..targets.len() {
        for j in i + 1..targets.len() {
            let (a, b) = (&targets[i], &targets[j]);
            if a.agent == b.agent
                || (settled.contains(&a.agent) && settled.contains(&b.agent))
                || (a.kind != b.kind && exclusive(&a.spec, &b.spec))
            {
                continue;
            }
            let same_folder = if a.folder == b.folder {
                Some(true)
            } else {
                let fa = lookup(
                    &mut looked.entry(a.agent.clone()).or_default().folder,
                    &a.folder,
                    now,
                    &a.agent,
                    Which::Folder,
                    ask,
                );
                let fb = lookup(
                    &mut looked.entry(b.agent.clone()).or_default().folder,
                    &b.folder,
                    now,
                    &b.agent,
                    Which::Folder,
                    ask,
                );
                match (fa, fb) {
                    (Some(x), Some(y)) => Some(x == y),
                    _ => None,
                }
            };
            let wait = |pending: &mut HashSet<AgentId>| {
                pending.insert(a.agent.clone());
                pending.insert(b.agent.clone());
            };
            match same_folder {
                Some(false) => continue,
                None => {
                    wait(&mut pending);
                    continue;
                }
                Some(true) => {}
            }
            let rival = if a.kind == b.kind {
                Some(true)
            } else {
                let ((la, wa), (lb, wb)) = (
                    session_root(&a.spec.glob, home),
                    session_root(&b.spec.glob, home),
                );
                if nested(&la, &lb) {
                    Some(true)
                } else {
                    let ra = lookup(
                        &mut looked.entry(a.agent.clone()).or_default().root,
                        &wa,
                        now,
                        &a.agent,
                        Which::Root,
                        ask,
                    );
                    let rb = lookup(
                        &mut looked.entry(b.agent.clone()).or_default().root,
                        &wb,
                        now,
                        &b.agent,
                        Which::Root,
                        ask,
                    );
                    match (ra, rb) {
                        (Some(Some(ra)), Some(Some(rb))) => Some(nested(&ra, &rb)),
                        // Cannot be known: fail closed.
                        (Some(None), Some(_)) | (Some(_), Some(None)) => Some(true),
                        _ => None,
                    }
                }
            };
            match rival {
                Some(true) => {
                    out.insert(a.agent.clone());
                    out.insert(b.agent.clone());
                }
                Some(false) => {}
                None => wait(&mut pending),
            }
        }
    }
    // A rival is settled whatever else is outstanding.
    pending.retain(|a| !out.contains(a));
    (out, pending)
}

impl SessionWatcher {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            waiting: HashMap::new(),
            gave_up: HashSet::new(),
            probe: workspace,
            resolver: resolve,
            looked: HashMap::new(),
            worker: None,
            seq: 0,
        }
    }

    /// Probes the disk on a thread of its own and delivers the answers by message (the node's supervision thread must never wait on
    /// a filesystem). Until an answer is there the agents concerned get no session and are not given up on.
    pub fn background(mut self) -> Self {
        let (ask_tx, ask_rx) = std::sync::mpsc::channel::<Ask>();
        let (answer_tx, answer_rx) = std::sync::mpsc::channel::<Answer>();
        let (probe, resolver) = (self.probe, self.resolver);
        let spawned = std::thread::Builder::new()
            .name("agora-session-probe".into())
            .spawn(move || {
                for a in ask_rx {
                    let value = match a.which {
                        Which::Folder => Some(probe(&a.path)),
                        Which::Root => resolver(&a.path),
                    };
                    let done = Answer {
                        agent: a.agent,
                        which: a.which,
                        seq: a.seq,
                        value,
                    };
                    if answer_tx.send(done).is_err() {
                        break;
                    }
                }
            });
        if spawned.is_ok() {
            self.worker = Some(Worker {
                tx: ask_tx,
                rx: answer_rx,
            });
        }
        self
    }

    fn apply(&mut self, a: Answer) {
        let Some(l) = self.looked.get_mut(&a.agent) else {
            return;
        };
        let slot = match a.which {
            Which::Folder => &mut l.folder,
            Which::Root => &mut l.root,
        };
        if let Some(s) = slot.as_mut().filter(|s| s.seq == a.seq) {
            s.value = Some(a.value);
        }
    }

    fn drain_probes(&mut self) {
        while let Some(a) = self.worker.as_ref().and_then(|w| w.rx.try_recv().ok()) {
            self.apply(a);
        }
    }

    /// Waits for every outstanding probe (tests).
    #[cfg(test)]
    fn settle_probes(&mut self) {
        while self
            .looked
            .values()
            .flat_map(|l| [&l.folder, &l.root])
            .flatten()
            .any(|s| s.value.is_none())
        {
            let Some(a) = self.worker.as_ref().and_then(|w| w.rx.recv().ok()) else {
                return;
            };
            self.apply(a);
        }
    }

    /// Looks for the session ids of `targets` (when due) and answers the ones found, each once. `known` are the session ids the node's
    /// agents already have: never given to another.
    pub fn poll(
        &mut self,
        now: Instant,
        targets: &[Target],
        known: &HashSet<String>,
    ) -> Vec<(AgentId, String)> {
        let live: HashSet<&AgentId> = targets.iter().map(|t| &t.agent).collect();
        self.waiting.retain(|a, _| live.contains(a));
        self.gave_up.retain(|a| live.contains(a));
        self.looked.retain(|a, _| live.contains(a));
        let Some(home) = self.home.clone() else {
            return Vec::new();
        };
        for t in targets {
            if self.gave_up.contains(&t.agent) {
                continue;
            }
            self.waiting
                .entry(t.agent.clone())
                .or_insert_with(|| Waiting {
                    since: SystemTime::now(),
                    give_up_at: now + GIVE_UP_AFTER,
                    next_scan: now,
                });
        }
        // Rivals in one folder could have written each other's files: none of them gets one. Decided from the manifests alone, before
        // any file is read, over every live target (a live agent can still write its file after discovery gave up on it), so the answer
        // is the same in every target order and whichever agents are due. Only worked out when some agent is due.
        let mut pending: HashSet<AgentId> = HashSet::new();
        if self
            .waiting
            .values()
            .any(|w| now >= w.next_scan || now >= w.give_up_at)
        {
            self.drain_probes();
            let (probe, resolver) = (self.probe, self.resolver);
            let (worker, seq) = (&self.worker, &mut self.seq);
            let mut ask = |agent: &AgentId, which: Which, path: &Path| {
                *seq += 1;
                match worker {
                    Some(w) => {
                        let sent = w.tx.send(Ask {
                            agent: agent.clone(),
                            which,
                            seq: *seq,
                            path: path.to_path_buf(),
                        });
                        // A probe that cannot even be sent stays unanswered: fail closed.
                        let _ = sent;
                        (*seq, None)
                    }
                    None => (
                        *seq,
                        Some(match which {
                            Which::Folder => Some(probe(path)),
                            Which::Root => resolver(path),
                        }),
                    ),
                }
            };
            let (rivals, p) = compare_rivals(
                targets,
                &home,
                &self.gave_up,
                &mut self.looked,
                now,
                &mut ask,
            );
            pending = p;
            for a in rivals {
                self.waiting.remove(&a);
                self.gave_up.insert(a);
            }
        }
        let mut found = Vec::new();
        let mut claimed: HashSet<String> = known.clone();
        for t in targets {
            let Some(w) = self.waiting.get(&t.agent) else {
                continue;
            };
            if now >= w.give_up_at {
                self.waiting.remove(&t.agent);
                self.gave_up.insert(t.agent.clone());
                continue;
            }
            if now < w.next_scan || pending.contains(&t.agent) {
                continue;
            }
            let since = w.since.checked_sub(SLACK).unwrap_or(SystemTime::UNIX_EPOCH);
            let glob = t.spec.glob.replacen("{home}", &home.to_string_lossy(), 1);
            let mut ids: Vec<String> = crate::usage_files::expand(Path::new(&glob))
                .into_iter()
                .filter(|p| {
                    std::fs::metadata(p)
                        .and_then(|m| m.created().or_else(|_| m.modified()))
                        .is_ok_and(|t| t >= since)
                })
                .filter_map(|p| session_in(&p, &t.spec, &t.folder))
                .filter(|id| !claimed.contains(id))
                .collect();
            ids.sort();
            ids.dedup();
            if let Some(w) = self.waiting.get_mut(&t.agent) {
                w.next_scan = now + SCAN_EVERY;
            }
            match ids.as_slice() {
                [] => {}
                [one] => {
                    claimed.insert(one.clone());
                    self.waiting.remove(&t.agent);
                    found.push((t.agent.clone(), one.clone()));
                }
                _ => {
                    self.waiting.remove(&t.agent);
                    self.gave_up.insert(t.agent.clone());
                }
            }
        }
        found
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_adapters::manifest::UsageSelect;

    /// `agora_platform::link_dir` with `/` spelled as the system's separator: Windows' `mklink` reads `h/link` as the switch `/link`.
    fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        let native = |p: &Path| p.components().collect::<PathBuf>();
        agora_platform::link_dir(&native(target), &native(link))
    }

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("agora-sessions-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("home/.tool/s")).unwrap();
        std::fs::create_dir_all(p.join("work/a")).unwrap();
        std::fs::create_dir_all(p.join("work/b")).unwrap();
        p
    }

    fn spec() -> SessionFile {
        SessionFile {
            glob: "{home}/.tool/s/rollout-*.jsonl".into(),
            select: Some(UsageSelect {
                path: "type".into(),
                equals: "session_meta".into(),
            }),
            id: "payload.id".into(),
            cwd: "payload.cwd".into(),
        }
    }

    fn write(root: &Path, name: &str, id: &str, cwd: &Path) {
        write_typed(root, name, "session_meta", id, cwd);
    }

    fn write_typed(root: &Path, name: &str, ty: &str, id: &str, cwd: &Path) {
        let line = serde_json::json!({"type": ty, "payload": {"id": id, "cwd": cwd}});
        std::fs::write(
            root.join("home/.tool/s").join(name),
            format!("{line}\n{{\"type\":\"other\"}}\n"),
        )
        .unwrap();
    }

    fn target(agent: &str, folder: PathBuf) -> Target {
        Target {
            agent: AgentId::new(agent),
            kind: "tool".into(),
            folder,
            spec: spec(),
        }
    }

    #[test]
    fn the_new_file_of_the_agents_folder_names_its_session_once() {
        let root = tmp("one");
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let t = vec![target("agent-1", root.join("work/a"))];
        let now = Instant::now();
        assert!(w.poll(now, &t, &HashSet::new()).is_empty());
        // Another folder's session, a line that is not a session start, and an id with characters argv must not get are not its.
        write(&root, "rollout-1.jsonl", "other-id", &root.join("work/b"));
        write(&root, "rollout-2.jsonl", "bad id;rm", &root.join("work/a"));
        write(
            &root,
            "rollout-3.jsonl",
            "0199aaaa-0000-4000-8000-000000000001",
            &root.join("work/a"),
        );
        let found = w.poll(now + SCAN_EVERY, &t, &HashSet::new());
        assert_eq!(
            found,
            vec![(
                AgentId::new("agent-1"),
                "0199aaaa-0000-4000-8000-000000000001".to_string()
            )]
        );
        // The node now knows it as that agent's: it is not offered again (the agent is no longer a target either).
        let known: HashSet<String> = ["0199aaaa-0000-4000-8000-000000000001".to_string()].into();
        assert!(w.poll(now + SCAN_EVERY * 2, &t, &known).is_empty());
        assert!(w.poll(now + SCAN_EVERY * 3, &[], &known).is_empty());
    }

    #[test]
    fn a_planted_file_can_never_put_an_option_on_the_agents_command_line() {
        let long = "a".repeat(129);
        let bad = [
            "--dangerously-bypass-approvals-and-sandbox",
            "-x",
            "a b",
            "a;b",
            "ünïcode-0000-4000-8000-000000000001",
            "",
            long.as_str(),
            // Argv-safe, but not a UUID: a session file's id must be one.
            "abc",
        ];
        for id in bad {
            let root = tmp("planted");
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let t = vec![target("agent-1", root.join("work/a"))];
            write(&root, "rollout-1.jsonl", id, &root.join("work/a"));
            assert!(
                w.poll(Instant::now(), &t, &HashSet::new()).is_empty(),
                "{id:?}"
            );
        }
        for id in &bad[..7] {
            assert!(!argv_safe(id), "{id:?}");
        }
        assert!(argv_safe("0199aaaa-0000-4000-8000-000000000001") && argv_safe("ses_9.x-Y"));
        assert!(argv_safe(&"a".repeat(128)));
    }

    #[test]
    fn two_agents_in_one_folder_or_two_fitting_files_get_no_session() {
        let root = tmp("two");
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let now = Instant::now();
        let twins = vec![
            target("agent-1", root.join("work/a")),
            target("agent-2", root.join("work/a")),
        ];
        write(
            &root,
            "rollout-1.jsonl",
            "0199aaaa-0000-4000-8000-000000000011",
            &root.join("work/a"),
        );
        assert!(w.poll(now, &twins, &HashSet::new()).is_empty());
        assert!(w.poll(now + SCAN_EVERY, &twins, &HashSet::new()).is_empty());
        // Alone in another folder, but two new files fit: no guess either.
        let mut w = SessionWatcher::new(Some(root.join("home")));
        write(
            &root,
            "rollout-2.jsonl",
            "0199aaaa-0000-4000-8000-000000000012",
            &root.join("work/b"),
        );
        write(
            &root,
            "rollout-3.jsonl",
            "0199aaaa-0000-4000-8000-000000000013",
            &root.join("work/b"),
        );
        let alone = vec![target("agent-3", root.join("work/b"))];
        assert!(w.poll(now, &alone, &HashSet::new()).is_empty());
        // A session another agent already has is never given again.
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let known: HashSet<String> = ["0199aaaa-0000-4000-8000-000000000012".to_string()].into();
        assert_eq!(
            w.poll(now, &alone, &known),
            vec![(
                AgentId::new("agent-3"),
                "0199aaaa-0000-4000-8000-000000000013".to_string()
            )]
        );
    }

    #[test]
    fn live_agents_whose_discovery_gave_up_keep_a_newcomers_session_ambiguous() {
        for reverse in [false, true] {
            let root = tmp(if reverse { "late-reversed" } else { "late" });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            let mut targets = vec![
                target("older-a", root.join("work/a")),
                target("older-b", root.join("work/a")),
            ];
            // Both older agents give up before a third agent starts or a file exists.
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                write(
                    &writer_root,
                    "rollout-late.jsonl",
                    "0199aaaa-0000-4000-8000-000000000091",
                    &writer_root.join("work/a"),
                );
            });
            targets.push(target("newcomer", root.join("work/a")));
            if reverse {
                targets.reverse();
            }
            assert!(w
                .poll(now + SCAN_EVERY, &targets, &HashSet::new())
                .is_empty());
            // Release the older agent's file only after the newcomer is registered.
            release.send(()).unwrap();
            writer.join().unwrap();
            assert!(w
                .poll(now + SCAN_EVERY * 2, &targets, &HashSet::new())
                .is_empty());
        }
    }

    #[test]
    fn a_live_timed_out_agent_still_owns_its_possible_session_until_it_leaves() {
        let root = tmp("expired-live");
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let now = Instant::now();
        let mut targets = vec![target("expired", root.join("work/a"))];
        assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
        let expired = now + GIVE_UP_AFTER;
        assert!(w.poll(expired, &targets, &HashSet::new()).is_empty());
        targets.push(target("newcomer", root.join("work/a")));
        assert!(w
            .poll(expired + SCAN_EVERY, &targets, &HashSet::new())
            .is_empty());
        write(
            &root,
            "rollout-late.jsonl",
            "0199aaaa-0000-4000-8000-000000000092",
            &root.join("work/a"),
        );
        assert!(w
            .poll(expired + SCAN_EVERY * 2, &targets, &HashSet::new())
            .is_empty());
        // Leaving the target set clears give-up state; a later unambiguous agent can discover its file.
        assert!(w
            .poll(expired + SCAN_EVERY * 3, &[], &HashSet::new())
            .is_empty());
        std::fs::remove_file(root.join("home/.tool/s/rollout-late.jsonl")).unwrap();
        let fresh = [target("fresh", root.join("work/a"))];
        assert!(w
            .poll(expired + SCAN_EVERY * 4, &fresh, &HashSet::new())
            .is_empty());
        write(
            &root,
            "rollout-fresh.jsonl",
            "0199aaaa-0000-4000-8000-000000000093",
            &root.join("work/a"),
        );
        assert_eq!(
            w.poll(expired + SCAN_EVERY * 5, &fresh, &HashSet::new()),
            vec![(
                fresh[0].agent.clone(),
                "0199aaaa-0000-4000-8000-000000000093".into()
            )]
        );
    }

    #[test]
    fn agents_of_two_kinds_reading_the_same_session_files_in_one_folder_get_no_session() {
        // F1180: a user's manifest copied from another kind names the same session files; a file cannot say which agent wrote it.
        for reverse in [false, true] {
            let root = tmp(if reverse { "kinds-reversed" } else { "kinds" });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            let mut targets = vec![
                target("tool-1", root.join("work/a")),
                Target {
                    kind: "tool-variant".into(),
                    ..target("variant-1", root.join("work/a"))
                },
            ];
            if reverse {
                targets.reverse();
            }
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                write(
                    &writer_root,
                    "rollout-1.jsonl",
                    "0199aaaa-0000-4000-8000-000000000095",
                    &writer_root.join("work/a"),
                );
            });
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            // The file appears only after both agents are registered.
            release.send(()).unwrap();
            writer.join().unwrap();
            assert!(w
                .poll(now + SCAN_EVERY, &targets, &HashSet::new())
                .is_empty());
            // In different folders each still gets its own.
            let mut w = SessionWatcher::new(Some(root.join("home")));
            targets[1].folder = root.join("work/b");
            let (a, b) = (targets[0].agent.clone(), targets[1].agent.clone());
            write(
                &root,
                "rollout-2.jsonl",
                "0199aaaa-0000-4000-8000-000000000096",
                &root.join("work/b"),
            );
            let mut found = w.poll(now, &targets, &HashSet::new());
            found.sort();
            let mut want = vec![
                (a, "0199aaaa-0000-4000-8000-000000000095".to_string()),
                (b, "0199aaaa-0000-4000-8000-000000000096".to_string()),
            ];
            want.sort();
            assert_eq!(found, want);
        }
    }

    #[test]
    fn agents_whose_globs_are_spelled_differently_but_match_the_same_file_get_no_session() {
        // Codex review of #791: `sessions/*.jsonl` and `sessions/./*.jsonl` name the same files.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "spelling-reversed"
            } else {
                "spelling"
            });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            let mut targets = vec![
                target("tool-1", root.join("work/a")),
                Target {
                    kind: "tool-variant".into(),
                    spec: SessionFile {
                        glob: "{home}/.tool/s/./rollout-*.jsonl".into(),
                        ..spec()
                    },
                    ..target("variant-1", root.join("work/a"))
                },
            ];
            if reverse {
                targets.reverse();
            }
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                write(
                    &writer_root,
                    "rollout-1.jsonl",
                    "0199aaaa-0000-4000-8000-000000000097",
                    &writer_root.join("work/a"),
                );
            });
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            // The file appears only after both agents are registered.
            release.send(()).unwrap();
            writer.join().unwrap();
            assert!(w
                .poll(now + SCAN_EVERY, &targets, &HashSet::new())
                .is_empty());
            // Neither takes it later either, even once the other agent has left.
            let alone = [targets[0].clone()];
            assert!(w
                .poll(now + SCAN_EVERY * 2, &alone, &HashSet::new())
                .is_empty());
        }
    }

    #[test]
    fn agents_reading_one_glob_with_exclusive_selectors_in_one_folder_each_get_their_own() {
        // Codex review of #791: no first line can pass both `select`s, so each file can only be one of them.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "selectors-reversed"
            } else {
                "selectors"
            });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            let mut targets = vec![
                target("tool-1", root.join("work/a")),
                Target {
                    kind: "tool-variant".into(),
                    spec: SessionFile {
                        select: Some(UsageSelect {
                            path: "type".into(),
                            equals: "variant_meta".into(),
                        }),
                        ..spec()
                    },
                    ..target("variant-1", root.join("work/a"))
                },
            ];
            if reverse {
                targets.reverse();
            }
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                let folder = writer_root.join("work/a");
                write_typed(
                    &writer_root,
                    "rollout-1.jsonl",
                    "session_meta",
                    "0199aaaa-0000-4000-8000-000000000098",
                    &folder,
                );
                write_typed(
                    &writer_root,
                    "rollout-2.jsonl",
                    "variant_meta",
                    "0199aaaa-0000-4000-8000-000000000099",
                    &folder,
                );
            });
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            release.send(()).unwrap();
            writer.join().unwrap();
            let mut found = w.poll(now + SCAN_EVERY, &targets, &HashSet::new());
            found.sort();
            let mut want = vec![
                (
                    AgentId::new("tool-1"),
                    "0199aaaa-0000-4000-8000-000000000098".to_string(),
                ),
                (
                    AgentId::new("variant-1"),
                    "0199aaaa-0000-4000-8000-000000000099".to_string(),
                ),
            ];
            want.sort();
            assert_eq!(found, want);
        }
    }

    fn with_glob(t: Target, kind: &str, glob: &str) -> Target {
        Target {
            kind: kind.into(),
            spec: SessionFile {
                glob: glob.into(),
                ..spec()
            },
            ..t
        }
    }

    /// Polls `targets` once to register them, writes `files` (name relative to the home, id, folder) only then, and polls again.
    fn found_after_gated_write(
        root: &Path,
        targets: &[Target],
        files: &[(&str, &str, &str)],
    ) -> Vec<(AgentId, String)> {
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let now = Instant::now();
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let writer_root = root.to_path_buf();
        let files: Vec<(String, String, String)> = files
            .iter()
            .map(|(n, i, f)| (n.to_string(), i.to_string(), f.to_string()))
            .collect();
        let writer = std::thread::spawn(move || {
            gate.recv().unwrap();
            for (name, id, folder) in files {
                let path = writer_root.join("home").join(&name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let line = serde_json::json!({"type": "session_meta",
                    "payload": {"id": id, "cwd": writer_root.join(folder)}});
                std::fs::write(path, format!("{line}\n")).unwrap();
            }
        });
        assert!(w.poll(now, targets, &HashSet::new()).is_empty());
        release.send(()).unwrap();
        writer.join().unwrap();
        let mut found = w.poll(now + SCAN_EVERY, targets, &HashSet::new());
        found.sort();
        found
    }

    #[test]
    fn agents_whose_globs_differ_under_one_session_root_get_no_session() {
        // Codex review of #791: `*/*.jsonl` and `*/rollout-*.jsonl` under one root can both match a file.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "one-root-reversed"
            } else {
                "one-root"
            });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/*/rollout-*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.tool/s/*/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let found = found_after_gated_write(
                &root,
                &targets,
                &[(
                    ".tool/s/2026/rollout-1.jsonl",
                    "0199aaaa-0000-4000-8000-000000000101",
                    "work/a",
                )],
            );
            assert_eq!(found, vec![]);
        }
    }

    #[test]
    fn agents_whose_session_roots_are_nested_get_no_session() {
        // One root inside the other: whether a file matches both depends on names the manifests do not fix, so neither gets one,
        // even for a file only one glob matches.
        for reverse in [false, true] {
            let root = tmp(if reverse { "nested-reversed" } else { "nested" });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/rollout-*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.tool/s/sub/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let found = found_after_gated_write(
                &root,
                &targets,
                &[
                    (
                        ".tool/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000102",
                        "work/a",
                    ),
                    (
                        ".tool/s/sub/other-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000103",
                        "work/a",
                    ),
                ],
            );
            assert_eq!(found, vec![]);
        }
    }

    #[test]
    fn agents_whose_session_roots_are_disjoint_each_get_their_own() {
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "disjoint-reversed"
            } else {
                "disjoint"
            });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/rollout-*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.other/s/rollout-*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let found = found_after_gated_write(
                &root,
                &targets,
                &[
                    (
                        ".tool/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000104",
                        "work/a",
                    ),
                    (
                        ".other/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000105",
                        "work/a",
                    ),
                ],
            );
            assert_eq!(
                found,
                vec![
                    (
                        AgentId::new("tool-1"),
                        "0199aaaa-0000-4000-8000-000000000104".to_string()
                    ),
                    (
                        AgentId::new("variant-1"),
                        "0199aaaa-0000-4000-8000-000000000105".to_string()
                    ),
                ]
            );
        }
    }

    #[test]
    fn who_gets_a_session_does_not_depend_on_target_order() {
        // Codex review of #791: the answer was decided while targets were walked, so their (HashMap) order could change it.
        let mut results = Vec::new();
        for reverse in [false, true] {
            let root = tmp(if reverse { "order-reversed" } else { "order" });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/rollout-*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.tool/s/sub/*.jsonl",
                ),
                with_glob(
                    target("other-1", root.join("work/a")),
                    "other",
                    "{home}/.other/s/rollout-*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            results.push(found_after_gated_write(
                &root,
                &targets,
                &[
                    (
                        ".tool/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000106",
                        "work/a",
                    ),
                    (
                        ".tool/s/sub/variant-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000107",
                        "work/a",
                    ),
                    (
                        ".other/s/rollout-1.jsonl",
                        "0199aaaa-0000-4000-8000-000000000108",
                        "work/a",
                    ),
                ],
            ));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(
            results[0],
            vec![(
                AgentId::new("other-1"),
                "0199aaaa-0000-4000-8000-000000000108".to_string()
            )]
        );
    }

    #[test]
    fn an_agent_not_due_is_still_found_a_rival_through_a_chain_of_rivals() {
        // Codex review of #791: A rivals B and B rivals C, A and C have exclusive selectors. With A and B due and C not, removing B
        // before it was compared with C let C later take B's session once B left.
        for reverse in [false, true] {
            let root = tmp(if reverse { "chain-reversed" } else { "chain" });
            let typed = |agent: &str, kind: &str, select: Option<&str>| Target {
                kind: kind.into(),
                spec: SessionFile {
                    select: select.map(|v| UsageSelect {
                        path: "type".into(),
                        equals: v.into(),
                    }),
                    ..spec()
                },
                ..target(agent, root.join("work/a"))
            };
            let (a, b, c) = (
                typed("a-1", "kind-a", Some("a_meta")),
                typed("b-1", "kind-b", None),
                typed("c-1", "kind-c", Some("c_meta")),
            );
            let mut w = SessionWatcher::new(Some(root.join("home")));
            let now = Instant::now();
            // C alone first: it scans, finds nothing, and is not due again for SCAN_EVERY.
            assert!(w
                .poll(now, std::slice::from_ref(&c), &HashSet::new())
                .is_empty());
            let mut all = vec![a.clone(), b.clone(), c.clone()];
            if reverse {
                all.reverse();
            }
            // A and B join and are due at once; C is not.
            assert!(w
                .poll(now + SCAN_EVERY / 3, &all, &HashSet::new())
                .is_empty());
            let (release, gate) = std::sync::mpsc::channel();
            let writer_root = root.clone();
            let writer = std::thread::spawn(move || {
                gate.recv().unwrap();
                // B writes its session; its tool's first line happens to pass C's select.
                write_typed(
                    &writer_root,
                    "rollout-b.jsonl",
                    "c_meta",
                    "0199aaaa-0000-4000-8000-000000000109",
                    &writer_root.join("work/a"),
                );
            });
            release.send(()).unwrap();
            writer.join().unwrap();
            // B leaves; C is due again and must not take B's session.
            let mut left = vec![a.clone(), c.clone()];
            if reverse {
                left.reverse();
            }
            assert_eq!(
                w.poll(now + SCAN_EVERY, &left, &HashSet::new()),
                vec![],
                "reverse={reverse}"
            );
        }
    }

    #[test]
    fn session_roots_resolve_links_before_their_folder_exists() {
        // Codex review of #791: `alias` links to `store`, `store/new` does not exist yet; `alias/new/*.jsonl` will find files that
        // `store/*/*.jsonl` finds, so the two are rivals already.
        for reverse in [false, true] {
            let root = tmp(if reverse { "alias-reversed" } else { "alias" });
            std::fs::create_dir_all(root.join("home/store")).unwrap();
            link_dir(&root.join("home/store"), &root.join("home/alias")).unwrap();
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/alias/new/*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/store/*/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let found = with_rivals(&targets, &root.join("home"));
            let want: HashSet<AgentId> = targets.iter().map(|t| t.agent.clone()).collect();
            assert_eq!(found, want, "reverse={reverse}");
        }
    }

    #[test]
    fn session_roots_follow_a_link_whose_target_does_not_exist_yet() {
        // Codex review of #791: `alias` links to `store/new` before `new` exists; `alias/*.jsonl` will find what `store/*/*.jsonl`
        // finds. The link is made while its target exists, then the target is removed (a dangling link on every platform).
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "dangling-reversed"
            } else {
                "dangling"
            });
            std::fs::create_dir_all(root.join("home/store/new")).unwrap();
            link_dir(&root.join("home/store/new"), &root.join("home/alias")).unwrap();
            std::fs::remove_dir(root.join("home/store/new")).unwrap();
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/alias/*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/store/*/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let want: HashSet<AgentId> = targets.iter().map(|t| t.agent.clone()).collect();
            assert_eq!(
                with_rivals(&targets, &root.join("home")),
                want,
                "reverse={reverse}"
            );
        }
    }

    #[test]
    fn a_root_that_cannot_be_resolved_makes_its_agent_a_rival_of_other_kinds_in_its_folder() {
        // Fail closed: too many links, or an error, leaves the root unknown.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "unknown-reversed"
            } else {
                "unknown"
            });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.lost/s/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let mut unknown =
                |p: &Path| (!p.to_string_lossy().contains(".lost")).then(|| p.to_path_buf());
            let want: HashSet<AgentId> = targets.iter().map(|t| t.agent.clone()).collect();
            assert_eq!(
                with_rivals_using(
                    &targets,
                    &root.join("home"),
                    &HashSet::new(),
                    &mut HashMap::new(),
                    &mut workspace,
                    &mut unknown
                ),
                want,
                "reverse={reverse}"
            );
        }
    }

    #[test]
    fn only_folders_shared_by_agents_of_different_kinds_have_their_roots_resolved() {
        // Codex review of #791: a root on a stalled mount must not hold up discovery for unrelated folders.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "resolve-reversed"
            } else {
                "resolve"
            });
            let mut targets = vec![
                // Alone in its folder: never compared, never resolved.
                with_glob(
                    target("lone-1", root.join("work/b")),
                    "lone",
                    "{home}/.lone/s/*.jsonl",
                ),
                // Two of one kind: rivals by kind, nothing to resolve.
                with_glob(
                    target("twin-1", root.join("work/c")),
                    "twin",
                    "{home}/.twin/s/*.jsonl",
                ),
                with_glob(
                    target("twin-2", root.join("work/c")),
                    "twin",
                    "{home}/.twin/s/*.jsonl",
                ),
                // Two kinds whose roots are disjoint as written: resolved to compare.
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/.tool/s/*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/.other/s/*.jsonl",
                ),
            ];
            std::fs::create_dir_all(root.join("work/c")).unwrap();
            if reverse {
                targets.reverse();
            }
            let mut seen: Vec<PathBuf> = Vec::new();
            let mut counting = |p: &Path| {
                seen.push(p.to_path_buf());
                Some(p.to_path_buf())
            };
            let found = with_rivals_using(
                &targets,
                &root.join("home"),
                &HashSet::new(),
                &mut HashMap::new(),
                &mut workspace,
                &mut counting,
            );
            let want: HashSet<AgentId> = [AgentId::new("twin-1"), AgentId::new("twin-2")].into();
            assert_eq!(found, want, "reverse={reverse}");
            let resolved: Vec<String> = seen
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            assert!(
                resolved
                    .iter()
                    .all(|p| !p.contains(".lone") && !p.contains(".twin")),
                "reverse={reverse}: {resolved:?}"
            );
            assert!(
                resolved.iter().any(|p| p.contains(".other")),
                "reverse={reverse}: {resolved:?}"
            );
        }
    }

    #[test]
    fn session_roots_spelled_in_another_case_are_rivals() {
        // Codex review of #791: on NTFS and APFS (case-insensitive by default) `Sessions` and `sessions` are one folder. Compared
        // without case everywhere: on a case-sensitive filesystem this only adds rivals (fails closed).
        for reverse in [false, true] {
            let root = tmp(if reverse { "case-reversed" } else { "case" });
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/Sessions/*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/sessions/new/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let want: HashSet<AgentId> = targets.iter().map(|t| t.agent.clone()).collect();
            assert_eq!(
                with_rivals(&targets, &root.join("home")),
                want,
                "reverse={reverse}"
            );
        }
    }

    thread_local! {
        static PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static PROBED: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn counting_probe(p: &Path) -> PathBuf {
        PROBES.with(|n| n.set(n.get() + 1));
        PROBED.with(|v| v.borrow_mut().push(p.to_path_buf()));
        workspace(p)
    }

    fn counting_resolver(p: &Path) -> Option<PathBuf> {
        PROBES.with(|n| n.set(n.get() + 1));
        PROBED.with(|v| v.borrow_mut().push(p.to_path_buf()));
        resolve(p)
    }

    #[test]
    fn each_agents_folder_and_session_root_are_looked_up_at_most_once() {
        // Codex review of #791: a stalled mount under one agent may delay at most the pass that first sees it, never every pass.
        for reverse in [false, true] {
            let root = tmp(if reverse { "once-reversed" } else { "once" });
            let mut w = SessionWatcher::new(Some(root.join("home")));
            w.probe = counting_probe;
            w.resolver = counting_resolver;
            PROBED.with(|v| v.borrow_mut().clear());
            let now = Instant::now();
            let mut targets: Vec<Target> = Vec::new();
            // Agents of different kinds arrive one per pass, each in one folder spelled its own way and with its own session root,
            // so every pair must be compared on disk.
            for i in 0..6u32 {
                let link = root.join(format!("work/l{i}"));
                link_dir(&root.join("work/a"), &link).unwrap();
                targets.push(with_glob(
                    target(&format!("agent-{i}"), link),
                    &format!("kind-{i}"),
                    &format!("{{home}}/r{i}/*.jsonl"),
                ));
                let mut order = targets.clone();
                if reverse {
                    order.reverse();
                }
                assert!(w
                    .poll(now + SCAN_EVERY * i, &order, &HashSet::new())
                    .is_empty());
            }
            let probed = PROBED.with(|v| v.borrow().clone());
            let mut counts: HashMap<PathBuf, usize> = HashMap::new();
            for p in &probed {
                *counts.entry(p.clone()).or_default() += 1;
            }
            assert!(
                counts.values().all(|n| *n == 1),
                "reverse={reverse}: {counts:?}"
            );
            // Every agent was looked at (its folder and its root). Their roots are disjoint, so none gave up: every pass compared
            // every pair again, and only the per-agent lookups kept each probe to one.
            assert_eq!(counts.len(), 12, "reverse={reverse}: {counts:?}");
            assert!(w.gave_up.is_empty(), "reverse={reverse}");
        }
    }

    #[test]
    fn an_existing_session_root_is_resolved_by_the_os() {
        // Codex review of #791: Windows takes `..` before a junction, Unix after a link; for a root that exists, the OS's own answer
        // (`canonicalize`) is used on every platform. CI runs this on Linux, macOS and Windows.
        let root = tmp("os-rules");
        std::fs::create_dir_all(root.join("real/inner")).unwrap();
        std::fs::create_dir_all(root.join("real/s")).unwrap();
        std::fs::create_dir_all(root.join("h/s")).unwrap();
        link_dir(&root.join("real/inner"), &root.join("h/link")).unwrap();
        let raw = root.join("h").join("link").join("..").join("s");
        assert_eq!(resolve(&raw), Some(std::fs::canonicalize(&raw).unwrap()));
        // A root that does not exist yet is walked, through the link.
        let missing = root.join("h").join("link").join("new");
        assert_eq!(
            resolve(&missing),
            Some(
                std::fs::canonicalize(root.join("real/inner"))
                    .unwrap()
                    .join("new")
            )
        );
    }

    #[test]
    fn agents_that_gave_up_together_are_not_probed_again_when_another_agent_is_due() {
        // Codex review of #791: a pair that already gave up must not be probed again (a stalled mount would block for nothing).
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "settled-reversed"
            } else {
                "settled"
            });
            link_dir(&root.join("work/a"), &root.join("work/link")).unwrap();
            let mut w = SessionWatcher::new(Some(root.join("home")));
            w.probe = counting_probe;
            w.resolver = counting_resolver;
            let now = Instant::now();
            // One kind, one folder spelled two ways: they give up together (probed once, as on main).
            let mut targets = vec![
                target("tool-1", root.join("work/a")),
                target("tool-2", root.join("work/link")),
            ];
            assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
            assert!(
                w.gave_up.contains(&AgentId::new("tool-1"))
                    && w.gave_up.contains(&AgentId::new("tool-2"))
            );
            // An unrelated agent (exclusive selector) arrives and is due: nothing is probed for the settled pair.
            targets.push(Target {
                kind: "tool-variant".into(),
                spec: SessionFile {
                    select: Some(UsageSelect {
                        path: "type".into(),
                        equals: "variant_meta".into(),
                    }),
                    ..spec()
                },
                ..target("variant-1", root.join("work/b"))
            });
            if reverse {
                targets.reverse();
            }
            PROBES.with(|n| n.set(0));
            assert!(w
                .poll(now + SCAN_EVERY / 3, &targets, &HashSet::new())
                .is_empty());
            assert_eq!(PROBES.with(|n| n.get()), 0, "reverse={reverse}");
            // The settled pair still counts against an agent that waits: a third of their kind in that folder gives up too.
            targets.push(target("tool-3", root.join("work/a")));
            assert!(w
                .poll(now + SCAN_EVERY / 2, &targets, &HashSet::new())
                .is_empty());
            assert!(
                w.gave_up.contains(&AgentId::new("tool-3")),
                "reverse={reverse}"
            );
        }
    }

    #[test]
    fn a_home_with_a_link_then_dotdot_is_resolved_as_the_filesystem_walks_it() {
        // Codex review of #791: `h/link/..` is `real` on disk (link -> real/inner), but `h` lexically. Two kinds whose roots meet only
        // in the real tree must be rivals.
        for reverse in [false, true] {
            let root = tmp(if reverse { "dotdot-reversed" } else { "dotdot" });
            std::fs::create_dir_all(root.join("real/inner")).unwrap();
            std::fs::create_dir_all(root.join("h")).unwrap();
            link_dir(&root.join("real/inner"), &root.join("h/link")).unwrap();
            let home = root.join("h").join("link").join("..");
            let real = root.join("real").join("s").join("sub").join("*.jsonl");
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/s/*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    &real.to_string_lossy(),
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let want: HashSet<AgentId> = targets.iter().map(|t| t.agent.clone()).collect();
            assert_eq!(with_rivals(&targets, &home), want, "reverse={reverse}");
        }
    }

    #[test]
    fn agents_of_one_kind_in_one_folder_spelled_two_ways_get_no_session() {
        // As on main (`same_dir`): `work/link` links to `work/a`, so an agent there and one in `work/a` share one folder.
        for reverse in [false, true] {
            let root = tmp(if reverse {
                "twin-link-reversed"
            } else {
                "twin-link"
            });
            link_dir(&root.join("work/a"), &root.join("work/link")).unwrap();
            let mut targets = vec![
                target("tool-1", root.join("work/a")),
                target("tool-2", root.join("work/link")),
            ];
            if reverse {
                targets.reverse();
            }
            let found = found_after_gated_write(
                &root,
                &targets,
                &[(
                    ".tool/s/rollout-1.jsonl",
                    "0199aaaa-0000-4000-8000-000000000111",
                    "work/a",
                )],
            );
            assert_eq!(found, vec![], "reverse={reverse}");
        }
    }

    #[test]
    fn no_folder_is_probed_unless_two_agents_could_contend() {
        // Codex review of #791: a stalled mount under a lone agent's folder must never block discovery. Agents that could contend (one
        // kind, or two kinds whose selectors are not exclusive) are probed, as `same_dir` always did.
        let root = tmp("no-probe");
        let lone = [target("lone-1", root.join("work/a"))];
        let same_kind = [
            target("tool-1", root.join("work/a")),
            target("tool-2", root.join("work/a")),
            target("tool-3", root.join("work/b")),
        ];
        let exclusive = [
            target("tool-1", root.join("work/a")),
            Target {
                kind: "tool-variant".into(),
                spec: SessionFile {
                    select: Some(UsageSelect {
                        path: "type".into(),
                        equals: "variant_meta".into(),
                    }),
                    ..spec()
                },
                ..target("variant-1", root.join("work/b"))
            },
        ];
        for (name, targets, want, probed) in [
            ("lone", &lone[..], vec![], false),
            ("same kind", &same_kind[..], vec!["tool-1", "tool-2"], true),
            ("exclusive", &exclusive[..], vec![], false),
        ] {
            for reverse in [false, true] {
                let mut targets = targets.to_vec();
                if reverse {
                    targets.reverse();
                }
                let mut probes: Vec<PathBuf> = Vec::new();
                let mut folder = |p: &Path| {
                    probes.push(p.to_path_buf());
                    p.to_path_buf()
                };
                let mut roots = |p: &Path| {
                    panic!("{name}: root {p:?} resolved");
                };
                let found = with_rivals_using(
                    &targets,
                    &root.join("home"),
                    &HashSet::new(),
                    &mut HashMap::new(),
                    &mut folder,
                    &mut roots,
                );
                let want: HashSet<AgentId> = want.iter().map(|a| AgentId::new(*a)).collect();
                assert_eq!(found, want, "{name} reverse={reverse}");
                assert_eq!(
                    !probes.is_empty(),
                    probed,
                    "{name} reverse={reverse}: {probes:?}"
                );
            }
        }
    }

    #[test]
    fn session_roots_that_meet_through_a_linked_folder_are_rivals() {
        // Codex review of #791: `logs/linked` links to `store`; `logs/*/*.jsonl` and `logs/linked/*.jsonl` find the same file, but
        // canonically the roots are `logs` and `store`.
        for reverse in [false, true] {
            let root = tmp(if reverse { "linked-reversed" } else { "linked" });
            std::fs::create_dir_all(root.join("home/logs")).unwrap();
            std::fs::create_dir_all(root.join("home/store")).unwrap();
            link_dir(&root.join("home/store"), &root.join("home/logs/linked")).unwrap();
            let mut targets = vec![
                with_glob(
                    target("tool-1", root.join("work/a")),
                    "tool",
                    "{home}/logs/*/*.jsonl",
                ),
                with_glob(
                    target("variant-1", root.join("work/a")),
                    "tool-variant",
                    "{home}/logs/linked/*.jsonl",
                ),
            ];
            if reverse {
                targets.reverse();
            }
            let found = found_after_gated_write(
                &root,
                &targets,
                &[(
                    "store/rollout-1.jsonl",
                    "0199aaaa-0000-4000-8000-000000000110",
                    "work/a",
                )],
            );
            assert_eq!(found, vec![], "reverse={reverse}");
        }
    }

    #[test]
    fn a_crlf_first_line_counts_like_an_lf_one_at_the_read_limit() {
        // Codex review of #791: the terminator is not part of the line, whether it is `\n` or `\r\n`.
        let root = tmp("crlf-limit");
        let folder = root.join("work/a");
        let id = "0199aaaa-0000-4000-8000-000000000100";
        let file = root.join("home/.tool/s/rollout-crlf.jsonl");
        let line =
            serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":folder}}).to_string();
        for length in [FIRST_LINE_MAX - 1, FIRST_LINE_MAX, FIRST_LINE_MAX + 1] {
            for end in ["\n", "\r\n"] {
                let mut padded = line.as_bytes().to_vec();
                padded.resize(length as usize, b' ');
                padded.extend_from_slice(end.as_bytes());
                padded.extend_from_slice(b"{\"type\":\"other\"}\r\n");
                std::fs::write(&file, &padded).unwrap();
                assert_eq!(
                    session_in(&file, &spec(), &folder),
                    (length <= FIRST_LINE_MAX).then(|| id.to_string()),
                    "{length} {end:?}"
                );
            }
        }
        // A carriage return that is not followed by a newline is part of an overlong line.
        let mut padded = line.as_bytes().to_vec();
        padded.resize(FIRST_LINE_MAX as usize, b' ');
        padded.extend_from_slice(b"\rx\n");
        std::fs::write(&file, &padded).unwrap();
        assert!(session_in(&file, &spec(), &folder).is_none());
    }

    #[test]
    fn a_session_first_line_must_fit_completely_within_the_read_limit() {
        let root = tmp("line-limit");
        let folder = root.join("work/a");
        let id = "0199aaaa-0000-4000-8000-000000000094";
        let file = root.join("home/.tool/s/rollout-limit.jsonl");
        let line =
            serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":folder}}).to_string();
        for length in [FIRST_LINE_MAX - 1, FIRST_LINE_MAX, FIRST_LINE_MAX + 1] {
            let mut padded = line.as_bytes().to_vec();
            padded.resize(length as usize, b' ');
            std::fs::write(&file, &padded).unwrap();
            assert_eq!(
                session_in(&file, &spec(), &folder),
                (length <= FIRST_LINE_MAX).then(|| id.to_string())
            );
        }
        // The prefix at the limit is valid JSON, but the complete physical line is not.
        let mut malformed = line.as_bytes().to_vec();
        malformed.resize(FIRST_LINE_MAX as usize, b' ');
        malformed.extend_from_slice(b"NOT_JSON\n");
        assert!(serde_json::from_slice::<Value>(&malformed).is_err());
        std::fs::write(&file, malformed).unwrap();
        assert!(session_in(&file, &spec(), &folder).is_none());
        // A bounded first line is accepted even when later lines are large.
        std::fs::write(
            &file,
            format!("{line}\n{}", "x".repeat(FIRST_LINE_MAX as usize * 2)),
        )
        .unwrap();
        assert_eq!(session_in(&file, &spec(), &folder), Some(id.to_string()));
    }

    #[test]
    fn a_workspace_moved_under_a_running_agent_is_found_again_when_its_old_path_becomes_a_link() {
        // #828: the first agent's folder was resolved while its path was a real folder; the folder was then moved and the old path
        // made a link to the new place. A second agent of the same kind started at the new path is in the same workspace.
        let root = tmp("moved");
        let mut w = SessionWatcher::new(Some(root.join("home")));
        let now = Instant::now();
        let (a, b) = (root.join("work/a"), root.join("work/q"));
        let alias = root.join("work/l");
        link_dir(&a, &alias).unwrap();
        let first = with_glob(target("first", a.clone()), "k1", "{home}/r1/*.jsonl");
        let other = with_glob(target("other", alias), "k2", "{home}/r2/*.jsonl");
        assert!(w
            .poll(now, &[first.clone(), other.clone()], &HashSet::new())
            .is_empty());
        assert!(w.gave_up.is_empty());
        std::fs::rename(&a, &b).unwrap();
        link_dir(&b, &a).unwrap();
        let second = with_glob(target("second", b), "k1", "{home}/r1/*.jsonl");
        let all = [first, other, second];
        w.poll(now + REVERIFY_AFTER + SCAN_EVERY, &all, &HashSet::new());
        assert!(w.gave_up.contains(&AgentId::new("first")));
        assert!(w.gave_up.contains(&AgentId::new("second")));
        assert!(!w.gave_up.contains(&AgentId::new("other")));
    }

    #[test]
    fn a_session_root_changed_by_a_reloaded_manifest_is_resolved_again() {
        // #828: the cache is by the expanded root too; a reload that points an agent's glob at another root is not compared with
        // the root it had before.
        let root = tmp("reload");
        let home = root.join("home");
        std::fs::create_dir_all(home.join("ra")).unwrap();
        std::fs::create_dir_all(home.join("rb")).unwrap();
        link_dir(&home.join("ra"), &home.join("rb2")).unwrap();
        let mut w = SessionWatcher::new(Some(home));
        let now = Instant::now();
        let a = with_glob(target("a", root.join("work/a")), "ka", "{home}/ra/*.jsonl");
        let b = with_glob(target("b", root.join("work/a")), "kb", "{home}/rb/*.jsonl");
        assert!(w.poll(now, &[a.clone(), b], &HashSet::new()).is_empty());
        assert!(w.gave_up.is_empty());
        let reloaded = with_glob(target("b", root.join("work/a")), "kb", "{home}/rb2/*.jsonl");
        w.poll(now + SCAN_EVERY, &[a, reloaded], &HashSet::new());
        assert_eq!(w.gave_up.len(), 2, "{:?}", w.gave_up);
    }

    static OPEN: (std::sync::Mutex<bool>, std::sync::Condvar) =
        (std::sync::Mutex::new(false), std::sync::Condvar::new());

    /// A probe of a mount that does not answer until the test opens the gate.
    fn stalled(p: &Path) -> Option<PathBuf> {
        let mut open = OPEN.0.lock().unwrap();
        while !*open {
            open = OPEN.1.wait(open).unwrap();
        }
        drop(open);
        resolve(p)
    }

    #[test]
    fn a_probe_that_does_not_answer_never_holds_up_the_caller_and_gives_no_session_meanwhile() {
        // #828: probes run off the supervision thread. `poll` returns while the probe is stuck (the test would hang otherwise), the
        // agents get no session although a file fits, and are not given up on; the answer arrives by message.
        let root = tmp("stalled");
        let home = root.join("home");
        std::fs::create_dir_all(home.join("ra")).unwrap();
        std::fs::create_dir_all(home.join("rb")).unwrap();
        let mut w = SessionWatcher::new(Some(home.clone()));
        w.resolver = stalled;
        let mut w = w.background();
        let now = Instant::now();
        let folder = root.join("work/a");
        let a = with_glob(target("a", folder.clone()), "ka", "{home}/ra/*.jsonl");
        let b = with_glob(target("b", folder.clone()), "kb", "{home}/rb/*.jsonl");
        let targets = [a, b];
        assert!(w.poll(now, &targets, &HashSet::new()).is_empty());
        let line = serde_json::json!({"type": "session_meta",
            "payload": {"id": "0199aaaa-0000-4000-8000-00000000000a", "cwd": folder}});
        std::fs::write(home.join("ra/x.jsonl"), format!("{line}\n")).unwrap();
        assert!(w
            .poll(now + SCAN_EVERY, &targets, &HashSet::new())
            .is_empty());
        assert!(w.gave_up.is_empty());
        assert_eq!(w.waiting.len(), 2);
        *OPEN.0.lock().unwrap() = true;
        OPEN.1.notify_all();
        w.settle_probes();
        let found = w.poll(now + SCAN_EVERY * 2, &targets, &HashSet::new());
        assert_eq!(
            found,
            vec![(
                AgentId::new("a"),
                "0199aaaa-0000-4000-8000-00000000000a".to_string()
            )]
        );
    }
}
