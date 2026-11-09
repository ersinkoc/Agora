//! Token usage read from the files an agent writes itself (ADR-0042). The manifest says where the file is and how to read its
//! lines (`agora-adapters::usage`); this module finds the files that belong to a running agent, follows their growth and returns
//! what each new line added. A file belongs to an agent when it was touched after the agent started and the working directory it
//! records is the agent's worktree, so another session of the same tool is never counted.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use agora_adapters::manifest::UsageFile;
use agora_adapters::usage::{FileReader, Sample, MAX_LINE};
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
/// A file may have been created this long before the node noticed the agent.
const SLACK: Duration = Duration::from_secs(2);

/// A running agent whose kind says where its usage file is.
pub struct Target {
    pub agent: AgentId,
    pub worktree: PathBuf,
    pub spec: UsageFile,
}

struct FileState {
    reader: FileReader,
    offset: u64,
    /// `None` until a line has said which directory the file belongs to.
    owned: Option<bool>,
    pending: Vec<Sample>,
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

/// The files a glob matches: `*` stands for any one path component's characters, nothing else is special.
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
        next.truncate(MAX_MATCHES);
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
        }
    }

    /// One step: follows the targets and returns what new lines added, per agent. Cheap when nothing is due.
    pub fn poll(&mut self, now: Instant, targets: &[Target]) -> Vec<(AgentId, Sample)> {
        let live: HashSet<&AgentId> = targets.iter().map(|t| &t.agent).collect();
        self.agents.retain(|id, _| live.contains(id));
        let mut out = Vec::new();
        let Some(home) = self.home.clone() else {
            return out;
        };
        for t in targets {
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
                    let fresh = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .is_ok_and(|m| m + SLACK >= a.first_seen);
                    if fresh {
                        a.files.insert(
                            path,
                            FileState {
                                reader: FileReader::new(t.spec.clone()),
                                offset: 0,
                                owned: None,
                                pending: Vec::new(),
                                partial: Vec::new(),
                            },
                        );
                    }
                }
            }
            if now < a.next_read {
                continue;
            }
            a.next_read = now + READ_EVERY;
            let mut budget = READ_BUDGET;
            let mut drop_files = Vec::new();
            for (path, f) in &mut a.files {
                if budget == 0 {
                    break;
                }
                let samples = read_new(path, f, &mut budget, &t.worktree, &t.spec);
                if f.owned == Some(false) {
                    drop_files.push(path.clone());
                }
                out.extend(samples.into_iter().map(|s| (t.agent.clone(), s)));
            }
            for p in drop_files {
                a.files.remove(&p);
                if a.ignored.len() < MAX_MATCHES {
                    a.ignored.insert(p);
                }
            }
        }
        out
    }
}

/// Reads what was appended to one file and returns the samples that are the agent's.
fn read_new(
    path: &Path,
    f: &mut FileState,
    budget: &mut u64,
    worktree: &Path,
    spec: &UsageFile,
) -> Vec<Sample> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map_or(0, |m| m.len());
    if len < f.offset {
        // Truncated or replaced: start over.
        *f = FileState {
            reader: FileReader::new(spec.clone()),
            offset: 0,
            owned: None,
            pending: Vec::new(),
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
        if f.owned.is_none() {
            if let Some(cwd) = f.reader.cwd() {
                f.owned = Some(same_dir(cwd, worktree));
                if f.owned == Some(true) {
                    out.append(&mut f.pending);
                }
            }
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
    out
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
}
