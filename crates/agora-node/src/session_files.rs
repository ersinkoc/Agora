//! A terminal agent's own session id, read from the session file the agent writes (ADR-0060, manifest `[session.file]`). Used for
//! agents whose id the node cannot choose (Codex: no flag sets it), so that Resume after a node restart can continue the conversation.
//!
//! A file is the agent's when it was written after the agent started, its first line passes `select`, names the agent's folder as its
//! working directory, and its id is no other agent's. This is a guess that can be wrong, so it is made only when it cannot be confused:
//! when two agents of one kind wait for their id in the same folder at once, or more than one new file fits, neither gets one (the
//! window then offers Start again, never a Resume into someone else's conversation). An agent that has no file after
//! [`GIVE_UP_AFTER`] gets none.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
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
    kind: String,
    folder: PathBuf,
    since: SystemTime,
    give_up_at: Instant,
    next_scan: Instant,
}

pub struct SessionWatcher {
    home: Option<PathBuf>,
    waiting: HashMap<AgentId, Waiting>,
    /// Agents that will get no id from a file (ambiguous, or none came in time), until they end.
    gave_up: HashSet<AgentId>,
}

impl Default for SessionWatcher {
    fn default() -> Self {
        Self::new(crate::usage_files::home_dir())
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

/// The id in `file`'s first line when that line is a session's start in `folder`.
fn session_in(file: &Path, spec: &SessionFile, folder: &Path) -> Option<String> {
    let f = std::fs::File::open(file).ok()?;
    let mut line = String::new();
    BufReader::new(f.take(FIRST_LINE_MAX))
        .read_line(&mut line)
        .ok()?;
    let v: Value = serde_json::from_str(line.trim_end()).ok()?;
    if let Some(sel) = &spec.select {
        if at(&v, &sel.path).and_then(Value::as_str) != Some(sel.equals.as_str()) {
            return None;
        }
    }
    let cwd = at(&v, &spec.cwd)?.as_str()?;
    if !same_dir(Path::new(cwd), folder) {
        return None;
    }
    let id = at(&v, &spec.id)?.as_str()?;
    (argv_safe(id) && is_uuid(id)).then(|| id.to_string())
}

impl SessionWatcher {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home,
            waiting: HashMap::new(),
            gave_up: HashSet::new(),
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
                    kind: t.kind.clone(),
                    folder: t.folder.clone(),
                    since: SystemTime::now(),
                    give_up_at: now + GIVE_UP_AFTER,
                    next_scan: now,
                });
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
            if now < w.next_scan {
                continue;
            }
            // Two agents of one kind waiting in one folder: a file cannot say which is whose.
            let crowded = self.waiting.iter().any(|(other, o)| {
                *other != t.agent && o.kind == w.kind && same_dir(&o.folder, &w.folder)
            });
            if crowded {
                let twins: Vec<AgentId> = self
                    .waiting
                    .iter()
                    .filter(|(_, o)| o.kind == w.kind && same_dir(&o.folder, &w.folder))
                    .map(|(a, _)| a.clone())
                    .collect();
                for a in twins {
                    self.waiting.remove(&a);
                    self.gave_up.insert(a);
                }
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
        let line = serde_json::json!({"type": "session_meta", "payload": {"id": id, "cwd": cwd}});
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
}
